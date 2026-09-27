use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

// Windows PE Authenticode constants
pub const IMAGE_DIRECTORY_ENTRY_SECURITY: usize = 4;
pub const WIN_CERT_REVISION_2_0: u16 = 0x0200;
pub const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticodeSignature {
    pub certificate_type: u16,
    pub revision: u16,
    pub signature_payload: Vec<u8>,
    pub cert_table_offset: u32,
    pub cert_table_size: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PeVerificationReport {
    pub sha256_digest: String,
    pub is_signed: bool,
    pub signature: Option<AuthenticodeSignature>,
    pub file_size: usize,
}

// Construct a synthetic signed PE binary with valid DOS header, PE header,
// Optional Header, and Authenticode Certificate Table.
pub fn create_synthetic_signed_pe(image_data: &[u8], cert_payload: &[u8]) -> Vec<u8> {
    let mut pe = Vec::new();

    // 1. DOS Header (64 bytes)
    pe.extend_from_slice(b"MZ"); // Magic
    pe.resize(0x3C, 0); // Pad to e_lfanew offset
    let pe_header_offset: u32 = 0x80;
    pe.extend_from_slice(&pe_header_offset.to_le_bytes()); // e_lfanew
    pe.resize(pe_header_offset as usize, 0);

    // 2. PE Header ('PE\0\0' + COFF header: 24 bytes)
    pe.extend_from_slice(b"PE\0\0");
    pe.extend_from_slice(&0x8664u16.to_le_bytes()); // Machine: x86_64
    pe.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections: 1
    pe.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    pe.extend_from_slice(&0u32.to_le_bytes()); // PointerToSymbolTable
    pe.extend_from_slice(&0u32.to_le_bytes()); // NumberOfSymbols
    let size_of_optional_header: u16 = 240;
    pe.extend_from_slice(&size_of_optional_header.to_le_bytes());
    pe.extend_from_slice(&0x0022u16.to_le_bytes()); // Characteristics: EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE

    // 3. Optional Header PE32+ (240 bytes)
    let optional_header_start = pe.len();
    pe.extend_from_slice(&0x020Bu16.to_le_bytes()); // Magic: PE32+ (64-bit)
    pe.resize(optional_header_start + 112, 0); // Jump to NumberOfRvaAndSizes offset
    pe.extend_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes: 16 data directories

    // Calculate offsets for image payload and certificate table
    let headers_total_size = optional_header_start + size_of_optional_header as usize + 40; // plus 1 section header
    let rounded_header_size = (headers_total_size + 511) & !511; // 512-byte aligned

    let image_offset = rounded_header_size;
    let image_len = image_data.len();
    let unaligned_cert_offset = image_offset + image_len;
    let cert_offset = (unaligned_cert_offset + 7) & !7; // 8-byte aligned for Authenticode
    let cert_entry_len = 8 + cert_payload.len(); // 8-byte WIN_CERTIFICATE header + payload
    let cert_table_size = (cert_entry_len + 7) & !7; // 8-byte aligned

    // Data Directories start at optional_header_start + 116
    // Entry 0..3: export, import, resource, exception (8 bytes each = 32 bytes)
    pe.resize(optional_header_start + 116 + 32, 0);

    // Entry 4: Security (Certificate Table: VirtualAddress = FileOffset, Size = cert_table_size)
    pe.extend_from_slice(&(cert_offset as u32).to_le_bytes());
    pe.extend_from_slice(&(cert_table_size as u32).to_le_bytes());

    // Pad remainder of optional header
    pe.resize(optional_header_start + size_of_optional_header as usize, 0);

    // 4. Section Header (.text, 40 bytes)
    pe.extend_from_slice(b".text\0\0\0");
    pe.extend_from_slice(&(image_len as u32).to_le_bytes()); // VirtualSize
    pe.extend_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress
    pe.extend_from_slice(&(image_len as u32).to_le_bytes()); // SizeOfRawData
    pe.extend_from_slice(&(image_offset as u32).to_le_bytes()); // PointerToRawData
    pe.resize(pe.len() + 16, 0); // Characteristics and relocations

    // Pad to image_offset
    pe.resize(image_offset, 0);

    // 5. Image code/data payload
    pe.extend_from_slice(image_data);

    // Pad to cert_offset (8-byte aligned)
    pe.resize(cert_offset, 0);

    // 6. WIN_CERTIFICATE structure
    // dwLength (u32), wRevision (u16), wCertificateType (u16), bCertificate (bytes)
    pe.extend_from_slice(&(cert_entry_len as u32).to_le_bytes());
    pe.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    pe.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    pe.extend_from_slice(cert_payload);

    // Final padding to cert_table_size alignment
    pe.resize(cert_offset + cert_table_size, 0);

    pe
}

pub fn verify_pe_authenticode(data: &[u8]) -> Result<PeVerificationReport> {
    if data.len() < 0x40 || &data[0..2] != b"MZ" {
        bail!("Invalid DOS header; not a valid PE file");
    }

    let e_lfanew = u32::from_le_bytes(
        data[0x3C..0x40]
            .try_into()
            .context("Failed reading e_lfanew")?,
    ) as usize;

    if data.len() < e_lfanew + 24 || &data[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        bail!("Invalid PE signature");
    }

    let optional_header_offset = e_lfanew + 24;
    let magic = u16::from_le_bytes(
        data[optional_header_offset..optional_header_offset + 2]
            .try_into()
            .context("Reading PE magic")?,
    );

    let (dir_offset, num_dirs) = if magic == 0x020B {
        // PE32+ (64-bit)
        let num_dirs_offset = optional_header_offset + 112;
        let num_dirs = u32::from_le_bytes(data[num_dirs_offset..num_dirs_offset + 4].try_into()?);
        (optional_header_offset + 116, num_dirs)
    } else if magic == 0x010B {
        // PE32 (32-bit)
        let num_dirs_offset = optional_header_offset + 96;
        let num_dirs = u32::from_le_bytes(data[num_dirs_offset..num_dirs_offset + 4].try_into()?);
        (optional_header_offset + 100, num_dirs)
    } else {
        bail!("Unknown PE magic: 0x{:04X}", magic);
    };

    let mut signature = None;
    if num_dirs > IMAGE_DIRECTORY_ENTRY_SECURITY as u32 {
        let sec_entry_offset = dir_offset + (IMAGE_DIRECTORY_ENTRY_SECURITY * 8);
        let cert_rva = u32::from_le_bytes(data[sec_entry_offset..sec_entry_offset + 4].try_into()?);
        let cert_size = u32::from_le_bytes(data[sec_entry_offset + 4..sec_entry_offset + 8].try_into()?);

        if cert_rva > 0 && cert_size > 8 {
            let cert_offset = cert_rva as usize;
            let cert_end = cert_offset + cert_size as usize;
            if cert_end <= data.len() {
                let dw_length = u32::from_le_bytes(data[cert_offset..cert_offset + 4].try_into()?);
                let w_revision = u16::from_le_bytes(data[cert_offset + 4..cert_offset + 6].try_into()?);
                let w_cert_type = u16::from_le_bytes(data[cert_offset + 6..cert_offset + 8].try_into()?);
                let payload_len = (dw_length as usize).saturating_sub(8);
                let payload = data[cert_offset + 8..cert_offset + 8 + payload_len].to_vec();

                signature = Some(AuthenticodeSignature {
                    certificate_type: w_cert_type,
                    revision: w_revision,
                    signature_payload: payload,
                    cert_table_offset: cert_rva,
                    cert_table_size: cert_size,
                });
            }
        }
    }

    let sha256_digest = format!("{:x}", Sha256::digest(data));
    let is_signed = signature.is_some();

    Ok(PeVerificationReport {
        sha256_digest,
        is_signed,
        signature,
        file_size: data.len(),
    })
}

pub fn verify_signature_preservation(
    pre_pack_data: &[u8],
    extracted_data: &[u8],
    binary_name: &str,
) -> Result<()> {
    let pre_report = verify_pe_authenticode(pre_pack_data)
        .with_context(|| format!("Validating pre-pack PE for {}", binary_name))?;
    let post_report = verify_pe_authenticode(extracted_data)
        .with_context(|| format!("Validating extracted PE for {}", binary_name))?;

    if !pre_report.is_signed {
        bail!("Pre-pack binary '{}' is not signed", binary_name);
    }
    if !post_report.is_signed {
        bail!(
            "Extracted binary '{}' lost its Authenticode signature during packaging",
            binary_name
        );
    }

    if pre_report.sha256_digest != post_report.sha256_digest {
        bail!(
            "Bit mismatch for '{}': pre-pack SHA-256 {} != extracted SHA-256 {}",
            binary_name,
            pre_report.sha256_digest,
            post_report.sha256_digest
        );
    }

    let pre_sig = pre_report.signature.unwrap();
    let post_sig = post_report.signature.unwrap();

    if pre_sig.signature_payload != post_sig.signature_payload {
        bail!(
            "Authenticode signature payload altered for '{}'",
            binary_name
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_pe_signing_and_verification() {
        let code = b"console application machine code here";
        let cert = b"Azure Trusted Signing Certificate PKCS7 Block for BoardPandas";
        let signed_pe = create_synthetic_signed_pe(code, cert);

        let report = verify_pe_authenticode(&signed_pe).expect("Must verify signed PE");
        assert!(report.is_signed);
        let sig = report.signature.expect("Must have signature");
        assert_eq!(sig.revision, WIN_CERT_REVISION_2_0);
        assert_eq!(sig.certificate_type, WIN_CERT_TYPE_PKCS_SIGNED_DATA);
        assert_eq!(sig.signature_payload, cert);

        // Verify preservation check succeeds on identical bytes
        verify_signature_preservation(&signed_pe, &signed_pe, "pandamux.exe")
            .expect("Preservation must succeed");
    }

    #[test]
    fn test_signature_preservation_fails_on_mutation() {
        let code = b"code";
        let cert = b"cert";
        let signed_pe = create_synthetic_signed_pe(code, cert);

        let mut mutated = signed_pe.clone();
        mutated[100] ^= 0xFF; // Flip bit in image

        let res = verify_signature_preservation(&signed_pe, &mutated, "pandamux.exe");
        assert!(res.is_err(), "Must fail when binary bytes are modified");
    }
}
