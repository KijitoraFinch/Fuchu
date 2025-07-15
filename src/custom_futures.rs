use crate::reactor;
use std::{
    future::Future,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::io::AsRawFd,
    pin::Pin,
    task::{Context, Poll},
};

pub struct AcceptFuture<'a> {
    listener: &'a TcpListener,
}

impl<'a> AcceptFuture<'a> {
    pub fn new(listener: &'a TcpListener) -> Self {
        listener.set_nonblocking(true).unwrap();
        Self { listener }
    }
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = io::Result<(TcpStream, std::net::SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.listener.accept() {
            Ok((stream, addr)) => Poll::Ready(Ok((stream, addr))),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                reactor().register(self.listener.as_raw_fd(), cx.waker().clone());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct ReadFuture<'a> {
    stream: &'a mut TcpStream,
    buf: &'a mut [u8],
}

impl<'a> ReadFuture<'a> {
    pub fn new(stream: &'a mut TcpStream, buf: &'a mut [u8]) -> Self {
        Self { stream, buf }
    }
}

impl<'a> Future for ReadFuture<'a> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this.stream.read(this.buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                reactor().register(this.stream.as_raw_fd(), cx.waker().clone());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct WriteFuture<'a> {
    stream: &'a mut TcpStream,
    buf: &'a [u8],
}

impl<'a> WriteFuture<'a> {
    pub fn new(stream: &'a mut TcpStream, buf: &'a [u8]) -> Self {
        Self { stream, buf }
    }
}

impl<'a> Future for WriteFuture<'a> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this.stream.write(this.buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                reactor().register(this.stream.as_raw_fd(), cx.waker().clone());
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

pub struct TimerFuture {
    deadline: std::time::Instant,
    timer_id: Option<usize>,
}

impl TimerFuture {
    pub fn new(duration: std::time::Duration) -> Self {
        Self {
            deadline: std::time::Instant::now() + duration,
            timer_id: None,
        }
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if std::time::Instant::now() >= self.deadline {
            Poll::Ready(())
        } else {
            if self.timer_id.is_none() {
                let id = reactor().register_timer(self.deadline, cx.waker().clone());
                self.timer_id = Some(id);
            }
            Poll::Pending
        }
    }
}
