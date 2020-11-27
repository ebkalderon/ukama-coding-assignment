use std::io;
use std::os::raw::c_int;
use std::os::unix::io::{FromRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub trait CommandExt {
    fn inherit_oci_pipes(&mut self, start_pipe: &PipeWriter, sync_pipe: &PipeReader) -> &mut Self;
}

impl CommandExt for tokio::process::Command {
    fn inherit_oci_pipes(&mut self, start_pipe: &PipeWriter, sync_pipe: &PipeReader) -> &mut Self {
        let start_fd = start_pipe.child_fd();
        let sync_fd = sync_pipe.child_fd();

        unsafe {
            self.env("_OCI_STARTPIPE", "3")
                .env("_OCI_SYNCPIPE", "4")
                .pre_exec(move || {
                    if libc::dup2(start_fd, 3) == -1 {
                        eprintln!("failed to duplicate start_fd");
                        return Err(std::io::Error::last_os_error());
                    }

                    if libc::dup2(sync_fd, 4) == -1 {
                        eprintln!("failed to duplicate sync_fd");
                        return Err(std::io::Error::last_os_error());
                    }

                    Ok(())
                })
        }
    }
}

/// Reader side of an inheritable pipe.
///
/// The child process will inherit the writer as a file descriptor.
#[derive(Debug)]
pub struct PipeReader {
    reader: File,
    child_fd: RawFd,
}

impl PipeReader {
    pub fn inheritable() -> io::Result<Self> {
        let (read_fd, write_fd) = create_pipe(Inheritable::Writer)?;
        Ok(PipeReader {
            reader: unsafe { File::from_raw_fd(read_fd) },
            child_fd: write_fd,
        })
    }

    pub fn child_fd(&self) -> RawFd {
        self.child_fd
    }
}

impl AsyncRead for PipeReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().reader).poll_read(cx, buf)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        unsafe { libc::close(self.child_fd) };
    }
}

/// Reader side of an inheritable pipe.
///
/// The child process will inherit the writer as a file descriptor.
#[derive(Debug)]
pub struct PipeWriter {
    writer: File,
    child_fd: RawFd,
}

impl PipeWriter {
    pub fn inheritable() -> io::Result<Self> {
        let (read_fd, write_fd) = create_pipe(Inheritable::Reader)?;
        Ok(PipeWriter {
            writer: unsafe { File::from_raw_fd(write_fd) },
            child_fd: read_fd,
        })
    }

    pub fn child_fd(&self) -> RawFd {
        self.child_fd
    }
}

impl AsyncWrite for PipeWriter {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().writer).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}

impl Drop for PipeWriter {
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
