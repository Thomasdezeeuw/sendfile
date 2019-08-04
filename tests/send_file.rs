use std::fs::File;
use std::future::Future;
use std::io::{self, Read};
use std::marker::Unpin;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::pin::Pin;
use std::sync::mpsc::{channel, Receiver};
use std::task::Poll;
use std::thread::{self, sleep, JoinHandle};
use std::time::Duration;

use futures_test::task::noop_context;

use send_file::send_file;

#[derive(Debug)]
struct TestFile {
    path: &'static str,
    data: &'static [u8],
}

macro_rules! test_file {
    ($file:expr) => {
        TestFile {
            path: concat!("tests/files/", $file),
            data: include_bytes!(concat!("files/", $file)),
        }
    };
}

const TEST_FILES: &'static [TestFile] = &[
    test_file!("hello_world.txt"),
    test_file!("lorem_ipsum_1.txt"),
    test_file!("lorem_ipsum_50.txt"),
    test_file!("lorem_ipsum_100.txt"),
    test_file!("lorem_ipsum_200.txt"),
    test_file!("lorem_ipsum_500.txt"),
    test_file!("lorem_ipsum_1000.txt"),
];

#[test]
fn tcp_blocking() -> io::Result<()> {
    let mut ctx = noop_context();
    let server = tcp_server(TEST_FILES.len())?;

    for test in TEST_FILES {
        let file = File::open(test.path)?;
        let stream = TcpStream::connect(server.address)?;

        let mut send_file = unsafe { send_file(file, stream) };

        // Should write the entire file in a single call in blocking mode.
        let result = Pin::new(&mut send_file).poll(&mut ctx)?;
        assert!(result.is_ready());
        assert_eq!(send_file.written(), test.data.len());

        let (_, socket) = send_file.into_inner();
        let local_address = socket.local_addr()?;
        drop(socket); // Close the socket.

        let (address, data) = server.send_files.recv().unwrap();
        assert_eq!(local_address, address);
        assert_eq!(data, test.data);
    }

    Ok(())
}

#[test]
fn tcp_blocking_non_blocking() -> io::Result<()> {
    let server = tcp_server(TEST_FILES.len())?;

    for test in TEST_FILES {
        let file = File::open(test.path)?;
        let stream = TcpStream::connect(server.address)?;
        stream.set_nonblocking(true)?;

        let mut send_file = unsafe { send_file(file, stream) };
        wait_loop(Pin::new(&mut send_file))?;
        assert_eq!(send_file.written(), test.data.len());

        let (_, socket) = send_file.into_inner();
        let local_address = socket.local_addr()?;
        drop(socket); // Close the socket.

        let (address, data) = server.send_files.recv().unwrap();
        assert_eq!(local_address, address);
        if data != test.data {
            panic!("Retrieved different data then expected for {}", test.path);
        }
    }

    Ok(())
}

/// A simple wait loop that completes the future.
fn wait_loop<Fut>(mut future: Pin<&mut Fut>) -> Fut::Output
where
    Fut: Future + Unpin,
{
    let mut ctx = noop_context();

    loop {
        if let Poll::Ready(val) = future.as_mut().poll(&mut ctx) {
            return val;
        }
        sleep(Duration::from_millis(10));
    }
}

struct TcpServer {
    /// Address the server is listening on.
    address: SocketAddr,
    /// Receiving end of all the files send to the server.
    send_files: Receiver<(SocketAddr, Vec<u8>)>,
    /// Thread handle, joined when this is dropped.
    handle: Option<JoinHandle<()>>,
}

impl Drop for TcpServer {
    fn drop(&mut self) {
        self.handle
            .take()
            .unwrap()
            .join()
            .expect("error waiting for TcpServer thread");
    }
}

/// Create a test TCP server.
fn tcp_server(n_connections: usize) -> io::Result<TcpServer> {
    let (sender, receiver) = channel();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;

    let handle = thread::spawn(move || {
        for _ in 0..n_connections {
            let (mut stream, address) = listener.accept().expect("unable to accept connection");
            let mut buf = Vec::new();
            stream
                .read_to_end(&mut buf)
                .expect("unable to read from connection");
            sender.send((address, buf)).expect("unable to send result");
        }
    });

    Ok(TcpServer {
        address,
        send_files: receiver,
        handle: Some(handle),
    })
}
