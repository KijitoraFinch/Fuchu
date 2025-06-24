use Fuchu::{Interest, Runtime};
use std::future::Future;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;

/// A future that waits for a file descriptor to become readable
struct ReadableFuture {
    fd: i32,
    completed: bool,
}

impl ReadableFuture {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            completed: false,
        }
    }
}

impl Future for ReadableFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completed {
            return Poll::Ready(Ok(()));
        }

        // Register with the reactor
        if let Err(e) =
            fuchu::Reactor::send_registration(self.fd, Interest::Readable, cx.waker().clone())
        {
            return Poll::Ready(Err(e));
        }

        self.completed = true;
        Poll::Pending
    }
}

/// A future that waits for a file descriptor to become writable
struct WritableFuture {
    fd: i32,
    completed: bool,
}

impl WritableFuture {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            completed: false,
        }
    }
}

impl Future for WritableFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completed {
            return Poll::Ready(Ok(()));
        }

        // Register with the reactor
        if let Err(e) =
            fuchu::Reactor::send_registration(self.fd, Interest::Writable, cx.waker().clone())
        {
            return Poll::Ready(Err(e));
        }

        self.completed = true;
        Poll::Pending
    }
}

#[test]
fn test_unix_socket_communication() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    // Create a temporary file for the Unix socket
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let socket_path = temp_file.path().to_path_buf();
    drop(temp_file); // Remove the file so we can create a socket there

    let result = Arc::new(Mutex::new(String::new()));
    let result_clone = result.clone();

    // Server task
    let socket_path_server = socket_path.clone();
    spawner.spawn(async move {
        let listener = UnixListener::bind(&socket_path_server).expect("Failed to bind Unix socket");

        // Accept a connection
        let (mut stream, _) = listener.accept().expect("Failed to accept connection");
        let fd = stream.as_raw_fd();

        // Wait for the stream to be readable
        ReadableFuture::new(fd)
            .await
            .expect("Failed to wait for readable");

        // Read data
        let mut buffer = [0u8; 1024];
        let n = stream
            .read(&mut buffer)
            .expect("Failed to read from stream");
        let received = String::from_utf8_lossy(&buffer[..n]);

        *result_clone.lock().unwrap() = received.to_string();
    });

    // Client task
    let socket_path_client = socket_path.clone();
    spawner.spawn(async move {
        // Give the server time to start listening
        thread::sleep(Duration::from_millis(50));

        let mut stream =
            UnixStream::connect(&socket_path_client).expect("Failed to connect to Unix socket");
        let fd = stream.as_raw_fd();

        // Wait for the stream to be writable
        WritableFuture::new(fd)
            .await
            .expect("Failed to wait for writable");

        // Write data
        stream
            .write_all(b"Hello from client!")
            .expect("Failed to write to stream");
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Wait for communication to complete
    thread::sleep(Duration::from_millis(500));

    assert_eq!(*result.lock().unwrap(), "Hello from client!");

    drop(spawner);

    // Clean up the socket file
    let _ = std::fs::remove_file(&socket_path);

    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_multiple_io_operations() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let results = Arc::new(Mutex::new(Vec::new()));

    // Create multiple Unix socket pairs
    for i in 0..3 {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let socket_path = temp_file.path().to_path_buf();
        drop(temp_file);

        let results_clone = results.clone();
        let socket_path_server = socket_path.clone();

        // Server task
        spawner.spawn(async move {
            let listener =
                UnixListener::bind(&socket_path_server).expect("Failed to bind Unix socket");

            let (mut stream, _) = listener.accept().expect("Failed to accept connection");
            let fd = stream.as_raw_fd();

            ReadableFuture::new(fd)
                .await
                .expect("Failed to wait for readable");

            let mut buffer = [0u8; 1024];
            let n = stream
                .read(&mut buffer)
                .expect("Failed to read from stream");
            let received = String::from_utf8_lossy(&buffer[..n]);

            results_clone.lock().unwrap().push(received.to_string());
        });

        // Client task
        let socket_path_client = socket_path.clone();
        spawner.spawn(async move {
            thread::sleep(Duration::from_millis(50));

            let mut stream =
                UnixStream::connect(&socket_path_client).expect("Failed to connect to Unix socket");
            let fd = stream.as_raw_fd();

            WritableFuture::new(fd)
                .await
                .expect("Failed to wait for writable");

            let message = format!("Message from client {}", i);
            stream
                .write_all(message.as_bytes())
                .expect("Failed to write to stream");
        });
    }

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Wait for all communications to complete
    thread::sleep(Duration::from_millis(1000));

    let results_vec = results.lock().unwrap();
    assert_eq!(results_vec.len(), 3);

    // Check that all messages were received (order might vary)
    let mut received_messages: Vec<String> = results_vec.clone();
    received_messages.sort();

    let mut expected_messages = vec![
        "Message from client 0".to_string(),
        "Message from client 1".to_string(),
        "Message from client 2".to_string(),
    ];
    expected_messages.sort();

    assert_eq!(received_messages, expected_messages);

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_io_registration_error_handling() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let error_occurred = Arc::new(Mutex::new(false));
    let error_clone = error_occurred.clone();

    spawner.spawn(async move {
        // Try to register an invalid file descriptor
        let result = ReadableFuture::new(-1).await;

        if result.is_err() {
            *error_clone.lock().unwrap() = true;
        }
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(200));

    // The future should have detected the error
    assert!(*error_occurred.lock().unwrap());

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_mixed_io_interests() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let socket_path = temp_file.path().to_path_buf();
    drop(temp_file);

    let results = Arc::new(Mutex::new(Vec::new()));
    let results_clone = results.clone();

    // Server that tests both readable and writable
    let socket_path_server = socket_path.clone();
    spawner.spawn(async move {
        let listener = UnixListener::bind(&socket_path_server).expect("Failed to bind Unix socket");

        let (mut stream, _) = listener.accept().expect("Failed to accept connection");
        let fd = stream.as_raw_fd();

        // Wait for writable first (should be immediate for a fresh connection)
        WritableFuture::new(fd)
            .await
            .expect("Failed to wait for writable");
        stream
            .write_all(b"Server message")
            .expect("Failed to write");

        // Then wait for readable
        ReadableFuture::new(fd)
            .await
            .expect("Failed to wait for readable");
        let mut buffer = [0u8; 1024];
        let n = stream.read(&mut buffer).expect("Failed to read");
        let received = String::from_utf8_lossy(&buffer[..n]);

        results_clone.lock().unwrap().push(received.to_string());
    });

    // Client
    let socket_path_client = socket_path.clone();
    spawner.spawn(async move {
        thread::sleep(Duration::from_millis(50));

        let mut stream = UnixStream::connect(&socket_path_client).expect("Failed to connect");
        let fd = stream.as_raw_fd();

        // Wait for readable first
        ReadableFuture::new(fd)
            .await
            .expect("Failed to wait for readable");
        let mut buffer = [0u8; 1024];
        let n = stream.read(&mut buffer).expect("Failed to read");
        let received = String::from_utf8_lossy(&buffer[..n]);

        assert_eq!(received, "Server message");

        // Then wait for writable and respond
        WritableFuture::new(fd)
            .await
            .expect("Failed to wait for writable");
        stream
            .write_all(b"Client response")
            .expect("Failed to write");
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(500));

    assert_eq!(*results.lock().unwrap(), vec!["Client response"]);

    drop(spawner);
    let _ = std::fs::remove_file(&socket_path);
    assert!(runtime_handle.join().is_ok());
}
