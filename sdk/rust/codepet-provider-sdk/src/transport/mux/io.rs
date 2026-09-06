use crate::transport::error::TransportError;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use std::{pin::Pin, task::{Context, Poll}, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

fn error(message: impl ToString) -> TransportError { TransportError::new(message) }

pub(super) struct Pipe<R, W> { pub(super) reader: R, pub(super) writer: W }
impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for Pipe<R, W> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}
impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for Pipe<R, W> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> { Pin::new(&mut self.writer).poll_write(cx, buf) }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Pin::new(&mut self.writer).poll_flush(cx) }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Pin::new(&mut self.writer).poll_shutdown(cx) }
}

// Validate the negotiated frame bound before Yamux allocates a DATA body. Yamux's
// built-in hard limit is intentionally larger than this profile's scheduling quantum.
pub(super) struct FrameBoundedIo<I> { inner: I, max_payload: usize, header: [u8; 12], filled: usize, sent: usize, remaining: usize,
    idle: Duration, progress: Option<Pin<Box<tokio::time::Sleep>>> }
impl<I> FrameBoundedIo<I> {
    pub(super) fn new(inner: I, max_payload: usize, idle: Duration) -> Self {
        Self { inner, max_payload, header: [0;12], filled: 0, sent: 0, remaining: 0, idle, progress: None }
    }
    fn progressed(&mut self) { self.progress = Some(Box::pin(tokio::time::sleep(self.idle))); }
}
impl<I: futures::AsyncRead + Unpin> futures::AsyncRead for FrameBoundedIo<I> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &mut [u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if bytes.is_empty() { return Poll::Ready(Ok(0)); }
        if let Some(timer) = &mut this.progress {
            if std::future::Future::poll(timer.as_mut(), cx).is_ready() {
                return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "incomplete Yamux frame stalled")));
            }
        }
        loop {
            if this.sent == 12 && this.remaining > 0 {
                let cap = bytes.len().min(this.remaining);
                let n = futures::ready!(Pin::new(&mut this.inner).poll_read(cx, &mut bytes[..cap]))?;
                if n == 0 { return Poll::Ready(Err(std::io::ErrorKind::UnexpectedEof.into())); }
                this.remaining -= n;
                if this.remaining == 0 { this.progress = None; } else { this.progressed(); }
                return Poll::Ready(Ok(n));
            }
            if this.sent == 12 { this.filled = 0; this.sent = 0; }
            while this.filled < 12 {
                let n = futures::ready!(Pin::new(&mut this.inner).poll_read(cx, &mut this.header[this.filled..]))?;
                if n == 0 { return Poll::Ready(if this.filled == 0 { Ok(0) } else { Err(std::io::ErrorKind::UnexpectedEof.into()) }); }
                this.filled += n; this.progressed();
                // Register the new deadline before the next underlying read can return Pending.
                if let Some(timer) = &mut this.progress { let _ = std::future::Future::poll(timer.as_mut(), cx); }
            }
            if this.sent == 0 && this.header[1] == 0 {
                this.remaining = u32::from_be_bytes(this.header[8..12].try_into().unwrap()) as usize;
                if this.remaining > this.max_payload { return Poll::Ready(Err(std::io::Error::other("DATA frame exceeds negotiated limit"))); }
            }
            let n = bytes.len().min(12 - this.sent);
            bytes[..n].copy_from_slice(&this.header[this.sent..this.sent+n]); this.sent += n;
            if this.sent == 12 && this.remaining == 0 { this.progress = None; }
            return Poll::Ready(Ok(n));
        }
    }
}
impl<I: futures::AsyncWrite + Unpin> futures::AsyncWrite for FrameBoundedIo<I> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<std::io::Result<usize>> { Pin::new(&mut self.inner).poll_write(cx, bytes) }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Pin::new(&mut self.inner).poll_flush(cx) }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> { Pin::new(&mut self.inner).poll_close(cx) }
}

pub(super) async fn timed_read(stream: &mut yamux::Stream, mut bytes: &mut [u8], idle: Duration) -> Result<(), TransportError> {
    while !bytes.is_empty() {
        let n = tokio::time::timeout(idle, stream.read(bytes)).await.map_err(|_| error("stream read idle timeout"))?.map_err(error)?;
        if n == 0 { return Err(error("stream ended before message completed")); }
        bytes = &mut bytes[n..];
    }
    Ok(())
}
pub(super) async fn timed_write(stream: &mut yamux::Stream, mut bytes: &[u8], idle: Duration) -> Result<(), TransportError> {
    while !bytes.is_empty() {
        let n = tokio::time::timeout(idle, stream.write(bytes)).await.map_err(|_| error("stream write idle timeout"))?.map_err(error)?;
        if n == 0 { return Err(error("stream closed during write")); }
        bytes = &bytes[n..];
    }
    tokio::time::timeout(idle, stream.flush()).await.map_err(|_| error("stream flush idle timeout"))?.map_err(error)
}
