use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::future::Future;
use std::pin::Pin;
use std::task::{self, Poll};
use std::ptr;
use std::marker::Unpin;

/// Send a `file` out a `socket`.
///
/// # Arguments
///
///  * `file` must be a regular file, i.e. [`File`], opened for reading.
///  * `socket` must be a socket, e.g. [`TcpStream`] or [`UdpSocket`], opened
///    for writing.
///
/// # Unsafety
///
/// This function is unsafe because the call must ensure that the provided
/// `file` and `socket` are usable in the `sendfile` system call. The
/// requirements for these different between platforms.
pub unsafe fn send_file<F, S>(file: F, socket: S) -> SendFile<F, S> {
    SendFile {
        file,
        socket,
        written: 0,
    }
}

/// TODO: doc waking isn't done.
pub struct SendFile<F, S> {
    file: F,
    socket: S,
    written: usize,
}

impl<F, S> SendFile<F, S> {

    /// Retrieve the file and socket.
    pub fn into_inner(self) -> (F, S) {
        (self.file, self.socket)
    }
}

#[cfg(target_os = "macos")]
impl<F, S> Future for SendFile<F, S>
    where F: AsRawFd + Unpin,
          S: AsRawFd + Unpin,
{
    /// The number of bytes written, or an I/O error.
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context) -> Poll<Self::Output> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        loop {
            match raw_send_file(file, socket) {
                Ok(n) => {
                    self.written += n;
                    // TODO: how to detect a partial write?
                    break Poll::Ready(Ok(self.written));
                },
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn raw_send_file(file: RawFd, socket: RawFd) -> io::Result<usize> {
    let mut result = 0;
    if unsafe { libc::sendfile(file, socket, 0, &mut result, ptr::null_mut(), 0) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result as usize)
    }
}
