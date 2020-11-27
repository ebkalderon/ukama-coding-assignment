//! Inheritable pipes for use with `conmon`.
//!
//! Rust does not provide an equivalent of Go's `cmd.ExtraFiles` by default, so this module
//! provides an equivalent.

use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{FromRawFd, RawFd};

use anyhow::{anyhow, Context};
use libc::pid_t;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const START_PIPE_FD: RawFd = 3;
const SYNC_PIPE_FD: RawFd = 4;

/// An extension trait for `tokio::process::Command`.
pub trait CommandExt {
    /// Configures the child process to accept `_OCI_STARTPIPE` and `_OCI_SYNCPIPE`.
    fn inherit_oci_pipes(&mut self, start: &StartPipe, sync: &SyncPipe) -> &mut Self;
}

impl CommandExt for tokio::process::Command {
    fn inherit_oci_pipes(&mut self, start: &StartPipe, sync: &SyncPipe) -> &mut Self {
        let start_fd = start.child_fd;
        let sync_fd = sync.child_fd;

        unsafe {
            self.env("_OCI_STARTPIPE", START_PIPE_FD.to_string())
                .env("_OCI_SYNCPIPE", SYNC_PIPE_FD.to_string())
                .pre_exec(move || {
                    if libc::dup2(start_fd, START_PIPE_FD) == -1 {
                        eprintln!("failed to duplicate start pipe file descriptor");
                        return Err(std::io::Error::last_os_error());
                    }

                    if libc::dup2(sync_fd, SYNC_PIPE_FD) == -1 {
                        eprintln!("failed to duplicate sync pipe file descriptor");
                        return Err(std::io::Error::last_os_error());
                    }

                    Ok(())
                })
        }
    }
}

/// A readable pipe for retrieving a container PID from `conmon`.
///
/// The write end of this pipe will be inherited by any spawned child processes.
#[derive(Debug)]
pub struct SyncPipe {
    reader: BufReader<File>,
    child_fd: RawFd,
}

impl SyncPipe {
    /// Creates a new `SyncPipe`, returning the read end to the user.
    ///
    /// Returns `Err` if an I/O error occurred.
    pub fn new() -> io::Result<Self> {
        let (read_fd, write_fd) = create_pipe(Inheritable::Writer)?;
        Ok(SyncPipe {
            reader: BufReader::new(unsafe { File::from_raw_fd(read_fd) }),
            child_fd: write_fd,
        })
    }

    /// Retrieves the `pid_t` of the spawned container from `conmon`.
    ///
    /// Returns `Err` if an I/O error occurred, or if spawning the container failed.
    pub async fn get_pid(&mut self) -> anyhow::Result<pid_t> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SyncInfo {
            Err { pid: pid_t, message: String },
            Ok { pid: pid_t },
        }

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .await
            .context("failed to read from SyncPipe")?;

        match serde_json::from_str(&line).context("failed to parse SyncInfo object")? {
            SyncInfo::Ok { pid } => Ok(pid),
            SyncInfo::Err { pid, message } => Err(anyhow!(
                "failed to read container PID from `conmon`, returned status {}: [{}]",
                pid,
                message
            )),
        }
    }
}

impl Drop for SyncPipe {
    fn drop(&mut self) {
        unsafe { libc::close(self.child_fd) };
    }
}

/// A writable pipe for signaling to `conmon` to begin setting up a container.
///
/// The read end of this pipe will be inherited by any spawned child processes.
#[derive(Debug)]
pub struct StartPipe {
    writer: File,
    child_fd: RawFd,
}

impl StartPipe {
    /// Creates a new `StartPipe`, returning the write end to the user.
    ///
    /// Returns `Err` if an I/O error occurred.
    pub fn new() -> io::Result<Self> {
        let (read_fd, write_fd) = create_pipe(Inheritable::Reader)?;
        Ok(StartPipe {
            writer: unsafe { File::from_raw_fd(write_fd) },
            child_fd: read_fd,
        })
    }

    /// Signals `conmon` to begin setting up the container.
    ///
    /// Returns `Err` if an I/O error occurred.
    pub async fn ready(mut self) -> anyhow::Result<()> {
        self.writer
            .write_all(&[0u8])
            .await
            .context("failed to send ready signal to `conmon`")
    }
}

impl Drop for StartPipe {
    fn drop(&mut self) {
        unsafe { libc::close(self.child_fd) };
    }
}

enum Inheritable {
    Reader,
    Writer,
}

fn create_pipe(kind: Inheritable) -> std::io::Result<(c_int, c_int)> {
    let mut fds = [-1 as c_int, -1 as c_int];

    if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
        return Err(std::io::Error::last_os_error());
    }

    let (inherit_idx, no_inherit_idx) = match kind {
        Inheritable::Reader => (0, 1),
        Inheritable::Writer => (1, 0),
    };

    let flags = match unsafe { libc::fcntl(fds[inherit_idx], libc::F_GETFD) } {
        -1 => return Err(std::io::Error::last_os_error()),
        value => value,
    };

    if unsafe { libc::fcntl(fds[no_inherit_idx], libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error());
    }

    Ok((fds[0], fds[1]))
}
