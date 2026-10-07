#![cfg(windows)]

use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

const MAX_PROCESS_IMAGE_CHARS: usize = 32_768;

/// Kernel-derived identity for the server behind an already-open named pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPipeServer {
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

/// Validates the process attached to the server end of a named-pipe handle.
///
/// The kernel-reported PID must match the signed runtime record when one is
/// available, and the process image must be the PandaMUX server executable.
pub fn validate_named_pipe_server<T: AsRawHandle>(
    pipe: &T,
    expected_process_id: Option<u32>,
) -> io::Result<ValidatedPipeServer> {
    let pipe_handle = pipe.as_raw_handle() as HANDLE;
    let mut process_id = 0u32;
    if unsafe { GetNamedPipeServerProcessId(pipe_handle, &mut process_id) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if process_id == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "named-pipe server did not expose a process id",
        ));
    }
    if let Some(expected) = expected_process_id
        && process_id != expected
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "named-pipe server process id {process_id} does not match runtime process id {expected}"
            ),
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
    validate_server_image(&image)?;

    Ok(ValidatedPipeServer {
        process_id,
        executable: image,
    })
}

fn validate_server_image(image: &str) -> io::Result<()> {
    let file_name = Path::new(image)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file_name.eq_ignore_ascii_case("pandamux-server.exe") {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("named-pipe server image is not approved: {file_name}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_server_control_image_is_accepted() {
        validate_server_image(r"C:\Program Files\PandaMUX\pandamux-server.exe")
            .expect("server image");
        validate_server_image(r"C:\PandaMUX\PANDAMUX-SERVER.EXE")
            .expect("case-insensitive server image");
        assert!(validate_server_image(r"C:\Temp\pandamux.exe").is_err());
        assert!(validate_server_image(r"C:\Windows\System32\powershell.exe").is_err());
    }
}
