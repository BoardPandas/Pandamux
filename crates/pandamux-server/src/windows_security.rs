#![cfg(windows)]

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};

const MAX_PROCESS_IMAGE_CHARS: usize = 32_768;

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
    let mut sid_text = null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_text) } == 0 {
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
