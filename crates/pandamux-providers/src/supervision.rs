use crate::error::ProviderError;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

/// Windows creation flag that suppresses console window popup for child processes.
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Creates a prepared `tokio::process::Command` configured with platform supervision flags.
pub fn create_supervised_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        // Place the child into its own process group so all descendants can be killed cleanly
        cmd.process_group(0);
    }

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    cmd
}

/// A supervised child process whose entire process tree is terminated on drop or exit.
pub struct SupervisedChild {
    child: Option<Child>,
    #[cfg(windows)]
    job_handle: isize,
    #[cfg(not(windows))]
    pgid: Option<u32>,
}

impl SupervisedChild {
    /// Spawns a command and attaches OS-level process tree supervision.
    pub fn spawn(mut cmd: Command) -> Result<Self, ProviderError> {
        let child = cmd.spawn().map_err(|e| ProviderError::ProcessFailed {
            program: format!("{:?}", cmd),
            message: e.to_string(),
        })?;

        #[cfg(windows)]
        {
            let job_handle = setup_windows_job_object(&child)?;
            Ok(Self {
                child: Some(child),
                job_handle,
            })
        }

        #[cfg(not(windows))]
        {
            let pgid = child.id();
            Ok(Self {
                child: Some(child),
                pgid,
            })
        }
    }

    /// Access mutable reference to the underlying child process.
    pub fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// Takes the child's stdin stream.
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.as_mut().and_then(|c| c.stdin.take())
    }

    /// Takes the child's stdout stream.
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.as_mut().and_then(|c| c.stdout.take())
    }

    /// Takes the child's stderr stream.
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.as_mut().and_then(|c| c.stderr.take())
    }

    /// Terminates the process tree immediately.
    pub async fn kill(&mut self) -> Result<(), ProviderError> {
        if let Some(mut child) = self.child.take() {
            #[cfg(windows)]
            {
                unsafe {
                    if self.job_handle != 0 {
                        windows_sys::Win32::System::JobObjects::TerminateJobObject(
                            self.job_handle as _,
                            1,
                        );
                    }
                }
            }

            #[cfg(not(windows))]
            {
                if let Some(pgid) = self.pgid {
                    let _ = nix_or_libc_kill_pgid(pgid);
                }
            }

            let _ = child.kill().await;
        }
        Ok(())
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if self.job_handle != 0 {
                unsafe {
                    windows_sys::Win32::System::JobObjects::TerminateJobObject(
                        self.job_handle as _,
                        1,
                    );
                    windows_sys::Win32::Foundation::CloseHandle(self.job_handle as _);
                }
                self.job_handle = 0;
            }
        }

        #[cfg(not(windows))]
        {
            if let Some(pgid) = self.pgid {
                let _ = nix_or_libc_kill_pgid(pgid);
            }
        }
    }
}

#[cfg(windows)]
fn setup_windows_job_object(child: &Child) -> Result<isize, ProviderError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job == std::ptr::null_mut() {
            return Err(ProviderError::SupervisionError {
                message: "CreateJobObjectW failed".to_string(),
            });
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let ret = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );

        if ret == 0 {
            CloseHandle(job);
            return Err(ProviderError::SupervisionError {
                message: "SetInformationJobObject failed".to_string(),
            });
        }

        if let Some(raw_handle) = child.raw_handle() {
            let assign_ret = AssignProcessToJobObject(job, raw_handle as _);
            if assign_ret == 0 {
                CloseHandle(job);
                return Err(ProviderError::SupervisionError {
                    message: "AssignProcessToJobObject failed".to_string(),
                });
            }
        }

        Ok(job as isize)
    }
}

#[cfg(not(windows))]
fn nix_or_libc_kill_pgid(pgid: u32) -> Result<(), ProviderError> {
    // Send SIGKILL (signal 9) to the process group (-pgid)
    let pid_val = -(pgid as i32);
    unsafe {
        c_kill(pid_val, 9);
    }
    Ok(())
}

#[cfg(not(windows))]
unsafe extern "C" {
    #[link_name = "kill"]
    fn c_kill(pid: i32, sig: i32) -> i32;
}
