use std::future::Future;
use std::marker::Unpin;
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::task::{self, Poll};
use std::{io, ptr};

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
/// This function is unsafe because the caller must ensure that the provided
/// `file` and `socket` are usable in the `sendfile` system call. The
/// requirements for this system call are different between platforms.
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
    #[cfg(any(target_os = "macos", target_os="freebsd"))]
    written: libc::off_t,
    #[cfg(target_os = "linux")]
    written: libc::ssize_t,
}

impl<F, S> SendFile<F, S> {
    /// Retrieve the file and socket.
    pub fn into_inner(self) -> (F, S) {
        (self.file, self.socket)
    }
}

impl<F, S> SendFile<F, S>
    where F: AsRawFd,
          S: AsRawFd,
{
    #[cfg(target_os = "macos")]
    fn raw_send_file(&mut self) -> io::Result<()> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        // On macOS `length` is value-result parameter. It determines the number
        // of bytes to write and return the number of bytes written also in case
        // of `EAGAIN` errors.
        let mut length = 0; // Send all bytes.
        let res = unsafe { libc::sendfile(file, socket, self.written, &mut length, ptr::null_mut(), 0) };
        self.written += length;
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn raw_send_file(&mut self) -> io::Result<()> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        // FIXME(Thomas): Not sure what will happend for files larger then this count.
        let count = libc::size_t::max_value() / 2;
        let n = unsafe { libc::sendfile(socket, file, ptr::null_mut(), count) };
        if n == -1 {
            Err(io::Error::last_os_error())
        } else {
            self.written += n;
            Ok(())
        }
    }

    #[cfg(target_os = "freebsd")]
    fn raw_send_file(&mut self) -> io::Result<()> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        let mut bytes_sent = 0;
        let res = unsafe {
            libc::sendfile(file, socket, self.written, 0,
                ptr::null_mut(), &mut bytes_sent, 0)
        };
        self.written += bytes_sent;
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os="freebsd"))]
impl<F, S> Future for SendFile<F, S>
    where F: AsRawFd + Unpin,
          S: AsRawFd + Unpin,
{
    /// The number of bytes written, or an I/O error.
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context) -> Poll<Self::Output> {
        loop {
            match self.raw_send_file() {
                Ok(()) => break Poll::Ready(Ok(self.written as usize)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue, // Try again.
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}
