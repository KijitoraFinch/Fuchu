use fuchu::custom_futures::TimerFuture;
use fuchu::{Runtime, RuntimeBuilder, Spawner};
use futures::channel::oneshot;
use std::os::fd::AsRawFd;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
};
use tracing::instrument;

#[test]
fn test_spawn_and_execute_trivial_future() {
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();

    let (sender, receiver) = oneshot::channel::<&'static str>();

    spawner.spawn(async move {
        sender.send("hello").unwrap();
    });

    let result = runtime.block_on(receiver).unwrap();
    assert_eq!(result, "hello");
}

use rand::Rng;
use std::time::{Duration, Instant};
#[test]
#[instrument]
fn test_asynchronous_scheduling() {
    // Initialize the tracing subscriber for logging
    tracing_subscriber::fmt::init();
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();
    // Create empty futures with random durations
    for i in 0..100 {
        let duration = Duration::from_millis(rand::thread_rng().gen_range(1..100));
        spawner.spawn(async move {
            tracing::info!("Starting future{:?} with duration {:?}", i, duration);
            TimerFuture::new(duration).await;
            tracing::info!("Future {:?} completed after {:?}", i, duration);
        });
    }
}
#[test]
fn test_echo_server_performance() {
    const NUM_REQUESTS: usize = 50000;

    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    spawner.clone().spawn(async move {
        loop {
            let (mut stream, _) = fuchu::custom_futures::AcceptFuture::new(&listener)
                .await
                .unwrap();
            spawner.spawn(async move {
                let mut buf = [0; 1024];
                let n = fuchu::custom_futures::ReadFuture::new(&mut stream, &mut buf)
                    .await
                    .unwrap();
                fuchu::custom_futures::WriteFuture::new(&mut stream, &buf[..n])
                    .await
                    .unwrap();
                fuchu::reactor().unregister(stream.as_raw_fd());
            });
        }
    });

    let mut latencies = Vec::with_capacity(NUM_REQUESTS);

    for _ in 0..NUM_REQUESTS {
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let start = Instant::now();
        client.write_all(b"hello").unwrap();
        let mut buf = [0; 1024];
        let n = client.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
        latencies.push(start.elapsed());
    }

    let total_duration: Duration = latencies.iter().sum();
    let avg_latency = total_duration / NUM_REQUESTS as u32;
    let min_latency = latencies.iter().min().unwrap();
    let max_latency = latencies.iter().max().unwrap();

    println!("\n--- Echo Server Performance ---");
    println!("Total requests: {}", NUM_REQUESTS);
    println!("Avg latency: {:?}", avg_latency);
    println!("Min latency: {:?}", min_latency);
    println!("Max latency: {:?}", max_latency);
    println!("-----------------------------\n");
}

// this test should panic
#[test]
#[should_panic()]
fn test_reactor_outside_runtime() {
    fuchu::reactor();
}
