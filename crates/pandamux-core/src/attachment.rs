use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::ids::ThreadId;

/// Size cap: maximum 10 MiB for image attachments.
pub const MAX_IMAGE_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// Size cap: maximum 25 MiB for general file/document attachments.
pub const MAX_FILE_SIZE_BYTES: u64 = 25 * 1024 * 1024;

/// Size cap: maximum 50 MiB total for all attachments in a single turn.
pub const MAX_TOTAL_ATTACHMENTS_PER_TURN_BYTES: u64 = 50 * 1024 * 1024;

/// Chunk size for chunked attachment transfer (256 KiB).
pub const ATTACHMENT_CHUNK_SIZE_BYTES: usize = 256 * 1024;

/// Record representing an imported or uploaded attachment on the hub/node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRecord {
    pub id: String,
    pub thread_id: ThreadId,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub file_path: String,
    pub created_at_ms: u64,
}

impl AttachmentRecord {
    /// Returns true if the attachment is an image MIME type.
    pub fn is_image(&self) -> bool {
        is_image_mime(&self.mime_type)
    }

    /// Formats human-readable file size (e.g., "120 B", "45.2 KiB", "3.4 MiB").
    pub fn human_size(&self) -> String {
        format_file_size(self.size_bytes)
    }
}

/// Formats a textual summary block of attachments for AI provider prompts.
pub fn format_attachments_summary(attachments: &[AttachmentRecord]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n[Attachments:\n");
    for att in attachments {
        s.push_str(&format!(
            "- {} ({}, {}): {}\n",
            att.file_name,
            att.mime_type,
            att.human_size(),
            att.file_path
        ));
    }
    s.push(']');
    s
}

/// Formats a byte count into a human-readable size string.
pub fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Returns true if the given MIME type represents an image.
pub fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

/// Determines the best-matching MIME type based on file path or extension.
pub fn detect_mime_type(path_or_filename: &str) -> &'static str {
    let path = Path::new(path_or_filename);
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("yaml") | Some("yml") => "text/yaml",
        Some("toml") => "text/plain",
        Some("md") | Some("markdown") => "text/markdown",
        Some("rs") => "text/x-rust",
        Some("ts") | Some("tsx") => "text/typescript",
        Some("js") | Some("jsx") => "text/javascript",
        Some("py") => "text/x-python",
        Some("html") | Some("htm") => "text/html",
        Some("css") => "text/css",
        Some("csv") => "text/csv",
        Some("txt") | Some("log") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Validation error for attachment size caps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentSizeError {
    ImageTooLarge { size: u64, limit: u64 },
    FileTooLarge { size: u64, limit: u64 },
    TotalTurnTooLarge { total: u64, limit: u64 },
}

impl std::fmt::Display for AttachmentSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ImageTooLarge { size, limit } => write!(
                f,
                "Image size ({}) exceeds limit of {}",
                format_file_size(*size),
                format_file_size(*limit)
            ),
            Self::FileTooLarge { size, limit } => write!(
                f,
                "File size ({}) exceeds limit of {}",
                format_file_size(*size),
                format_file_size(*limit)
            ),
            Self::TotalTurnTooLarge { total, limit } => write!(
                f,
                "Total attachments size ({}) exceeds turn limit of {}",
                format_file_size(*total),
                format_file_size(*limit)
            ),
        }
    }
}

impl std::error::Error for AttachmentSizeError {}

/// Validates an attachment's size against individual image/file caps.
pub fn validate_attachment_size(
    mime_type: &str,
    size_bytes: u64,
) -> Result<(), AttachmentSizeError> {
    if is_image_mime(mime_type) {
        if size_bytes > MAX_IMAGE_SIZE_BYTES {
            return Err(AttachmentSizeError::ImageTooLarge {
                size: size_bytes,
                limit: MAX_IMAGE_SIZE_BYTES,
            });
        }
    } else if size_bytes > MAX_FILE_SIZE_BYTES {
        return Err(AttachmentSizeError::FileTooLarge {
            size: size_bytes,
            limit: MAX_FILE_SIZE_BYTES,
        });
    }
    Ok(())
}

/// Validates total size across multiple attachments for a single turn.
pub fn validate_total_attachments_size(
    attachments: &[AttachmentRecord],
) -> Result<(), AttachmentSizeError> {
    let total: u64 = attachments.iter().map(|a| a.size_bytes).sum();
    if total > MAX_TOTAL_ATTACHMENTS_PER_TURN_BYTES {
        return Err(AttachmentSizeError::TotalTurnTooLarge {
            total,
            limit: MAX_TOTAL_ATTACHMENTS_PER_TURN_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mime_type() {
        assert_eq!(detect_mime_type("photo.PNG"), "image/png");
        assert_eq!(detect_mime_type("document.pdf"), "application/pdf");
        assert_eq!(detect_mime_type("code.rs"), "text/x-rust");
        assert_eq!(detect_mime_type("unknown.bin"), "application/octet-stream");
    }

    #[test]
    fn test_validate_attachment_size() {
        // Image under 10 MiB is valid
        assert!(validate_attachment_size("image/png", 5 * 1024 * 1024).is_ok());
        // Image over 10 MiB errors
        assert!(validate_attachment_size("image/png", 11 * 1024 * 1024).is_err());

        // File under 25 MiB is valid
        assert!(validate_attachment_size("application/pdf", 20 * 1024 * 1024).is_ok());
        // File over 25 MiB errors
        assert!(validate_attachment_size("application/pdf", 26 * 1024 * 1024).is_err());
    }

    #[test]
    fn test_validate_total_attachments_size() {
        let att1 = AttachmentRecord {
            id: "att-1".into(),
            thread_id: ThreadId::from("t-1"),
            file_name: "f1.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 20 * 1024 * 1024,
            file_path: "/tmp/f1.pdf".into(),
            created_at_ms: 1000,
        };
        let att2 = AttachmentRecord {
            id: "att-2".into(),
            thread_id: ThreadId::from("t-1"),
            file_name: "f2.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 20 * 1024 * 1024,
            file_path: "/tmp/f2.pdf".into(),
            created_at_ms: 1000,
        };
        assert!(validate_total_attachments_size(&[att1.clone(), att2.clone()]).is_ok());

        let att3 = AttachmentRecord {
            id: "att-3".into(),
            thread_id: ThreadId::from("t-1"),
            file_name: "f3.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 15 * 1024 * 1024,
            file_path: "/tmp/f3.pdf".into(),
            created_at_ms: 1000,
        };
        // 20 + 20 + 15 = 55 MiB > 50 MiB
        assert!(validate_total_attachments_size(&[att1, att2, att3]).is_err());
    }
}
