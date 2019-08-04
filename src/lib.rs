//! Crate that wraps the `sendfile` system call.
//!
//! To create a new [`SendFile`] [`Future`] see [`send_file`].

#![warn(
    anonymous_parameters,
    bare_trait_objects,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    variant_size_differences
)]
// Disallow warnings when running tests.
#![cfg_attr(test, deny(warnings))]
// Disallow warnings in examples.
#![doc(test(attr(deny(warnings))))]

use std::future::Future;
use std::io;
use std::marker::Unpin;
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::task::{self, Poll};

/// Send a `file` out a `socket`.
///
/// # Arguments
///
///  * `file` must be a regular file, i.e. [`File`], opened for reading.
///  * `socket` must be a socket, e.g. [`TcpStream`] or [`UdpSocket`], opened
///    for writing.
///
/// [`File`]: std::fs::File
/// [`TcpStream`]: std::net::TcpStream
/// [`UdpSocket`]: std::net::UdpSocket
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

/// Wrapper around the `sendfile` system call.
///
/// From the `sendfile` manual (`$ man sendfile 2`):
///
/// > `sendfile` copies data between one file descriptor and another. Because
/// > this copying is done within the kernel, `sendfile` is more efficient than
/// > the combination of read and write, which would require transferring data
/// > to and from user space.
///
/// # Platform support
///
/// * Android.
/// * [FreeBSD](https://www.freebsd.org/cgi/man.cgi?query=sendfile&manpath=FreeBSD+12.0-RELEASE+and+Ports).
/// * [Linux](http://man7.org/linux/man-pages/man2/sendfile.2.html).
/// * [macOS](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/sendfile.2.html).
///
/// # Notes
///
/// The [`Future`] implementation doesn't implement waking, it is up to the
/// caller to ensure future is polled again once the socket is ready to receive
/// more data.
#[derive(Debug)]
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

    /// Returns the number of bytes written.
    pub fn written(&self) -> usize {
        self.written
    }
}

impl<F, S> SendFile<F, S>
where
    F: AsRawFd,
    S: AsRawFd,
{
    #[cfg(target_os = "macos")]
    fn raw_send_file(&mut self) -> io::Result<usize> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        // On macOS `length` is value-result parameter. It determines the number
        // of bytes to write and returns the number of bytes written also in
        // case of `EAGAIN` errors.
        let mut length = 0; // Send all bytes.
        let res = unsafe {
            libc::sendfile(
                file,
                socket,
                self.written as libc::off_t,
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        self.written += length as usize;
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(length as usize)
        }
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn raw_send_file(&mut self) -> io::Result<usize> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        // This is the maximum the Linux kernel will write in a single call.
        let count = 0x7ffff000;
        let mut offset = self.written as libc::off_t;
        let n = unsafe { libc::sendfile(socket, file, &mut offset, count) };
        if n == -1 {
            Err(io::Error::last_os_error())
        } else {
            self.written = offset as usize;
            Ok(n as usize)
        }
    }

    #[cfg(target_os = "freebsd")]
    fn raw_send_file(&mut self) -> io::Result<usize> {
        let file = self.file.as_raw_fd();
        let socket = self.socket.as_raw_fd();
        let mut bytes_sent = 0;
        let res = unsafe {
            libc::sendfile(
                file,
                socket,
                self.written as libc::off_t,
                0,
                std::ptr::null_mut(),
                &mut bytes_sent,
                0,
            )
        };
        self.written += bytes_sent as usize;
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(bytes_sent as usize)
        }
    }
}

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "macos",
))]
impl<F, S> Future for SendFile<F, S>
where
    F: AsRawFd + Unpin,
    S: AsRawFd + Unpin,
{
    /// The number of bytes written, or an I/O error.
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        loop {
            match self.raw_send_file() {
                Ok(0) => break Poll::Ready(Ok(())),
                Ok(_) => continue, // Attempt to write some more bytes.
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue, // Try again.
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}
