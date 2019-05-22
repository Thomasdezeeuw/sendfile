use std::fs::File;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::io::{self, Read};
use std::thread::{self, JoinHandle};
use std::net::{SocketAddr, TcpStream, TcpListener};
use std::sync::mpsc::{channel, Receiver};

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

        let result = Pin::new(&mut send_file).poll(&mut ctx)?;
        assert_eq!(result, Poll::Ready(test.data.len()));
        let (_, socket) = send_file.into_inner();
        let local_address = socket.local_addr()?;
        drop(socket); // Close the socket.

        let (address, data) = server.send_files.recv().unwrap();
        assert_eq!(local_address, address);
        assert_eq!(data, test.data);
    }

    Ok(())
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
        self.handle.take().unwrap().join().expect("error waiting for TcpServer thread");
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
            stream.read_to_end(&mut buf).expect("unable to read from connection");
            sender.send((address, buf)).expect("unable to send result");
        }
    });

    Ok(TcpServer {
        address,
        send_files: receiver,
        handle: Some(handle),
    })
}
