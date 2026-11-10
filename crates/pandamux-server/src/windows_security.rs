#![cfg(windows)]

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, GetAce,
    GetSecurityDescriptorControl, GetTokenInformation, OBJECT_INHERIT_ACE,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ALL_ACCESS};
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};

const MAX_PROCESS_IMAGE_CHARS: usize = 32_768;
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
const BUILTIN_ADMINISTRATORS_SID: &str = "S-1-5-32-544";

pub struct ValidatedPipeClient {
    pub process_id: u32,
    pub executable: String,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl OwnedSecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let wide = sddl.encode_utf16().chain([0]).collect::<Vec<_>>();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    fn into_raw(self) -> PSECURITY_DESCRIPTOR {
        let descriptor = std::mem::ManuallyDrop::new(self);
        descriptor.0
    }
}

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0 as HLOCAL);
            }
        }
    }
}

/// Owns a protected DACL granting pipe access only to the current user SID.
pub struct UserOnlySecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl UserOnlySecurity {
    pub fn for_current_user() -> io::Result<Self> {
        Self::for_sid(&current_user_sid()?)
    }

    fn for_sid(sid: &str) -> io::Result<Self> {
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let descriptor = OwnedSecurityDescriptor::from_sddl(&sddl)?.into_raw();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        (&mut self.attributes as *mut SECURITY_ATTRIBUTES).cast()
    }
}

/// Creates one directory with a protected, inheritable DACL that grants access
/// only to the current user. The caller validates the resulting object so an
/// existing path or a creation race cannot bypass the ownership boundary.
pub(crate) fn create_private_directory(path: &Path) -> io::Result<()> {
    let sid = current_user_sid()?;
    let descriptor = OwnedSecurityDescriptor::from_sddl(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})"))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(error);
        }
    }
    validate_private_path(path, true)
}

/// Validates a fixed managed root. Existing roots may inherit access for the
/// current user and Windows recovery principals. Newly created roots use the
/// stricter protected descriptor above.
pub(crate) fn validate_private_directory(path: &Path) -> io::Result<()> {
    validate_private_path(path, false)
}

/// Validates a descendant of a managed root. Descendants may carry inherited
/// entries, but every access-allow ACE and the owner must remain the current
/// user or a Windows recovery principal.
pub(crate) fn validate_private_descendant(path: &Path) -> io::Result<()> {
    validate_private_path(path, false)
}

fn validate_private_path(path: &Path, require_protected: bool) -> io::Result<()> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = OwnedSecurityDescriptor(descriptor);
    validate_private_security_descriptor(descriptor.0, owner, dacl, require_protected)
}

fn validate_private_security_descriptor(
    descriptor: PSECURITY_DESCRIPTOR,
    owner: PSID,
    dacl: *mut ACL,
    require_protected: bool,
) -> io::Result<()> {
    if descriptor.is_null() || owner.is_null() || dacl.is_null() {
        return Err(private_path_error("missing owner or DACL"));
    }
    let current_sid = current_user_sid()?;
    let owner_sid = sid_string(owner)?;
    if !trusted_path_principal(&owner_sid, &current_sid) {
        return Err(private_path_error(
            "owner is not the current user or a Windows recovery principal",
        ));
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if require_protected && control & SE_DACL_PROTECTED == 0 {
        return Err(private_path_error("DACL inherits from an unmanaged parent"));
    }

    let mut current_user_full_control = false;
    let ace_count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(ace_count) {
        let mut raw_ace: *mut c_void = null_mut();
        if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
        if u32::from(ace.Header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
            return Err(private_path_error("DACL contains an unsupported ACE"));
        }
        let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast::<c_void>();
        let ace_sid = sid_string(ace_sid)?;
        if !trusted_path_principal(&ace_sid, &current_sid) {
            return Err(private_path_error(
                "DACL grants access to another principal",
            ));
        }
        if ace_sid == current_sid && ace.Mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS {
            return Err(private_path_error(
                "current user does not retain complete managed-path access",
            ));
        }
        if require_protected
            && u32::from(ace.Header.AceFlags) & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)
                != (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)
        {
            return Err(private_path_error(
                "managed root DACL does not protect descendants",
            ));
        }
        current_user_full_control |= ace_sid == current_sid;
    }
    if !current_user_full_control {
        return Err(private_path_error("current user has no allow ACE"));
    }
    Ok(())
}

fn trusted_path_principal(candidate: &str, current_sid: &str) -> bool {
    candidate == current_sid
        || candidate == LOCAL_SYSTEM_SID
        || candidate == BUILTIN_ADMINISTRATORS_SID
}

fn private_path_error(detail: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("managed path is not private to the current user: {detail}"),
    )
}

impl Drop for UserOnlySecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                LocalFree(self.descriptor as HLOCAL);
            }
        }
    }
}

pub fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);

    let mut required = 0u32;
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let word_size = size_of::<usize>();
    let mut buffer = vec![0usize; (required as usize).div_ceil(word_size)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    sid_string(token_user.User.Sid)
}

fn sid_string(sid: PSID) -> io::Result<String> {
    if sid.is_null() {
        return Err(private_path_error("security descriptor has no SID"));
    }
    let mut sid_text = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut sid_text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid = unsafe {
        let mut length = 0usize;
        while *sid_text.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, length))
    };
    unsafe {
        LocalFree(sid_text as HLOCAL);
    }
    Ok(sid)
}

pub fn pipe_name_for_sid(role: &str, sid: &str) -> String {
    format!(r"\\.\pipe\pandamux-{role}-{sid}")
}

pub fn secure_pipe_name(role: &str) -> io::Result<String> {
    Ok(pipe_name_for_sid(role, &current_user_sid()?))
}

/// Reads and validates the connected named-pipe client before routing any bytes.
pub fn validate_named_pipe_client<T: AsRawHandle>(pipe: &T) -> io::Result<ValidatedPipeClient> {
    let pipe_handle = pipe.as_raw_handle() as HANDLE;
    let mut process_id = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe_handle, &mut process_id) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if process_id == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "named-pipe client did not expose a process id",
        ));
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = OwnedHandle(process);
    let mut image = vec![0u16; MAX_PROCESS_IMAGE_CHARS];
    let mut image_len = image.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.0, 0, image.as_mut_ptr(), &mut image_len) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let image = String::from_utf16_lossy(&image[..image_len as usize]);
    validate_client_image(&image)?;
    Ok(ValidatedPipeClient {
        process_id,
        executable: image,
    })
}

fn validate_client_image(image: &str) -> io::Result<()> {
    let file_name = Path::new(image)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if ["pandamux.exe", "pandamux-cli.exe"]
        .iter()
        .any(|allowed| file_name.eq_ignore_ascii_case(allowed))
    {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("named-pipe client image is not approved: {file_name}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_sid_scoped_and_role_specific() {
        let sid = "S-1-5-21-100-200-300-400";
        let hub = pipe_name_for_sid("hub", sid);
        let node = pipe_name_for_sid("node", sid);
        assert!(hub.contains(sid));
        assert!(node.contains(sid));
        assert_ne!(hub, node);
    }

    #[test]
    fn only_product_control_images_are_accepted() {
        validate_client_image(r"C:\Program Files\PandaMUX\pandamux.exe").expect("desktop image");
        validate_client_image(r"C:\Program Files\PandaMUX\PANDAMUX-CLI.EXE").expect("CLI image");
        assert!(validate_client_image(r"C:\Temp\unrelated.exe").is_err());
    }

    #[test]
    fn private_directory_security_rejects_foreign_owner_and_broad_access() {
        let sid = current_user_sid().expect("current user SID");
        let safe = OwnedSecurityDescriptor::from_sddl(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})"))
            .expect("safe descriptor");
        let (safe_owner, safe_dacl) = descriptor_owner_and_dacl(safe.0);
        validate_private_security_descriptor(safe.0, safe_owner, safe_dacl, true)
            .expect("safe descriptor validates");

        let foreign_owner =
            OwnedSecurityDescriptor::from_sddl(&format!("O:BUD:P(A;OICI;FA;;;{sid})"))
                .expect("foreign-owner descriptor");
        let (foreign_owner_sid, foreign_owner_dacl) = descriptor_owner_and_dacl(foreign_owner.0);
        assert!(
            validate_private_security_descriptor(
                foreign_owner.0,
                foreign_owner_sid,
                foreign_owner_dacl,
                true,
            )
            .is_err()
        );

        let broad = OwnedSecurityDescriptor::from_sddl(&format!(
            "O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FR;;;WD)"
        ))
        .expect("broad descriptor");
        let (broad_owner, broad_dacl) = descriptor_owner_and_dacl(broad.0);
        assert!(
            validate_private_security_descriptor(broad.0, broad_owner, broad_dacl, true,).is_err()
        );
    }

    #[test]
    fn private_directory_creation_applies_validated_security() {
        let temp = tempfile::tempdir().expect("temporary parent");
        let private = temp.path().join("private");
        create_private_directory(&private).expect("create private directory");
        validate_private_path(&private, true).expect("validate protected private directory");
    }

    fn descriptor_owner_and_dacl(descriptor: PSECURITY_DESCRIPTOR) -> (PSID, *mut ACL) {
        use windows_sys::Win32::Security::{GetSecurityDescriptorDacl, GetSecurityDescriptorOwner};

        let mut owner = null_mut();
        let mut owner_defaulted = 0;
        assert_ne!(
            unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) },
            0,
            "descriptor owner"
        );
        let mut dacl_present = 0;
        let mut dacl = null_mut();
        let mut dacl_defaulted = 0;
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(
                    descriptor,
                    &mut dacl_present,
                    &mut dacl,
                    &mut dacl_defaulted,
                )
            },
            0,
            "descriptor DACL"
        );
        assert_ne!(dacl_present, 0, "descriptor has a DACL");
        (owner, dacl)
    }

    #[tokio::test]
    async fn first_pipe_instance_rejects_anti_squatting_collision() {
        use tokio::net::windows::named_pipe::ServerOptions;

        let sid = current_user_sid().expect("current user SID");
        let path = format!(
            "{}-anti-squatting-{}",
            pipe_name_for_sid("test", &sid),
            std::process::id()
        );
        let mut security = UserOnlySecurity::for_sid(&sid).expect("user-only security");
        let mut first_options = ServerOptions::new();
        first_options
            .first_pipe_instance(true)
            .reject_remote_clients(true);
        let first = unsafe {
            first_options.create_with_security_attributes_raw(&path, security.as_mut_ptr())
        }
        .expect("first protected pipe instance");

        let mut colliding_options = ServerOptions::new();
        colliding_options
            .first_pipe_instance(true)
            .reject_remote_clients(true);
        let collision = unsafe {
            colliding_options.create_with_security_attributes_raw(&path, security.as_mut_ptr())
        };
        assert!(
            collision.is_err(),
            "a second first-instance server must not acquire the pipe name"
        );
        drop(first);
    }
}
