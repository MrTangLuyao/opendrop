//! Connection IO with progress deadlines, so that a peer which stops
//! reading or sending cannot hold a connection (and the buffers queued for
//! it) forever.
//!
//! - A write that stays pending for `write_stall` fails: the peer has not
//!   taken a single byte in that time. Any completed write restarts the
//!   clock, so a slow but steady reader is never cut off.
//! - A read that stays pending fails once nothing has moved in either
//!   direction for `read_idle`. While a response is being written the
//!   deadline keeps moving (hyper watches the read side for EOF during a
//!   download), so only a connection that is idle both ways times out. The
//!   handlers apply their own, shorter limits to request bodies; this one is
//!   the backstop.

use std::future::Future;
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

pub struct TimeoutIo<T> {
    inner: T,
    write_stall: Duration,
    read_idle: Duration,
    /// Last time bytes moved in either direction.
    progress: Instant,
    read_timer: Option<Pin<Box<Sleep>>>,
    write_timer: Option<Pin<Box<Sleep>>>,
}

impl<T> TimeoutIo<T> {
    pub fn new(inner: T, write_stall: Duration, read_idle: Duration) -> Self {
        TimeoutIo {
            inner,
            write_stall,
            read_idle,
            progress: Instant::now(),
            read_timer: None,
            write_timer: None,
        }
    }

    fn read_pending<R>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<R>> {
        let deadline = self
            .progress
            .checked_add(self.read_idle)
            .unwrap_or(self.progress);
        let timer = self
            .read_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if timer.deadline() != deadline {
            timer.as_mut().reset(deadline);
        }
        match timer.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.read_timer = None;
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "connection idle",
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn write_pending<R>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<R>> {
        let stall = self.write_stall;
        // the clock starts at the first pending write; only a completed
        // write stops it
        let timer = self
            .write_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(stall)));
        match timer.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.write_timer = None;
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "peer stopped reading",
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn wrote<R>(&mut self, r: io::Result<R>, moved: bool) -> Poll<io::Result<R>> {
        self.write_timer = None;
        if moved {
            self.progress = Instant::now();
        }
        Poll::Ready(r)
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for TimeoutIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(r) => {
                this.read_timer = None;
                if buf.filled().len() > before {
                    this.progress = Instant::now();
                }
                Poll::Ready(r)
            }
            Poll::Pending => this.read_pending(cx),
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for TimeoutIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, data) {
            Poll::Ready(r) => {
                let moved = matches!(r, Ok(n) if n > 0);
                this.wrote(r, moved)
            }
            Poll::Pending => this.write_pending(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(r) => {
                let moved = matches!(r, Ok(n) if n > 0);
                this.wrote(r, moved)
            }
            Poll::Pending => this.write_pending(cx),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(r) => this.wrote(r, false),
            Poll::Pending => this.write_pending(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(r) => this.wrote(r, false),
            Poll::Pending => this.write_pending(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn stalled_writer_fails_and_steady_reader_does_not() {
        let (a, b) = tokio::io::duplex(1024);
        let mut io = TimeoutIo::new(a, Duration::from_millis(300), Duration::from_secs(60));
        // nobody reads b: the write stalls once the pipe is full
        let big = vec![7u8; 64 * 1024];
        let err = io.write_all(&big).await.expect_err("stall must fail");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);

        // a reader that takes a little every 100 ms keeps the writer alive
        // far past the stall limit
        let (a, mut b2) = tokio::io::duplex(1024);
        let mut io = TimeoutIo::new(a, Duration::from_millis(300), Duration::from_secs(60));
        let reader = tokio::spawn(async move {
            let mut got = 0usize;
            let mut buf = [0u8; 512];
            while got < 16 * 1024 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                match b2.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got += n,
                }
            }
            got
        });
        io.write_all(&vec![1u8; 16 * 1024])
            .await
            .expect("slow but steady");
        assert_eq!(reader.await.expect("join"), 16 * 1024);
        drop(b);
    }

    #[tokio::test]
    async fn idle_read_fails_unless_writes_move() {
        let (a, b) = tokio::io::duplex(1 << 20);
        let mut io = TimeoutIo::new(a, Duration::from_secs(60), Duration::from_millis(300));
        let mut buf = [0u8; 16];
        let err = io.read(&mut buf).await.expect_err("idle read");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);

        // writes in the other direction keep a pending read alive
        let (a, mut b2) = tokio::io::duplex(1 << 20);
        let io = TimeoutIo::new(a, Duration::from_secs(60), Duration::from_millis(300));
        let (mut rd, mut wr) = tokio::io::split(io);
        let writer = tokio::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if wr.write_all(b"x").await.is_err() {
                    return false;
                }
            }
            true
        });
        let reader =
            tokio::spawn(async move { rd.read(&mut [0u8; 4]).await.map_err(|e| e.kind()) });
        assert!(writer.await.expect("join"));
        b2.write_all(b"done").await.expect("write");
        assert_eq!(reader.await.expect("join"), Ok(4));
        drop(b);
    }
}
