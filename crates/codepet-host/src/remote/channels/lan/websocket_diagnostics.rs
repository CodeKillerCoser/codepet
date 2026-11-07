use super::super::webrtc::diagnostics::Diagnostic;
use futures_util::{Sink, Stream};
use serde_json::json;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use yawc::{
    frame::{FrameView, OpCode},
    WebSocket, WebSocketError,
};

/// yawc's Stream adapter discards read errors. Preserve a bounded classification
/// before retaining the same end-of-stream behavior. Never log frame contents.
pub(super) struct ObservedSocket {
    socket: WebSocket,
    diagnostic: Arc<Diagnostic>,
    ended: bool,
}
impl ObservedSocket {
    pub fn new(socket: WebSocket) -> Self {
        let diagnostic = Diagnostic::new(&uuid::Uuid::new_v4().to_string());
        diagnostic.emit("lan.open", json!({}));
        Self {
            socket,
            diagnostic,
            ended: false,
        }
    }
    fn close_frame(&self, side: &str, frame: &FrameView) {
        if frame.opcode == OpCode::Close {
            let code = frame
                .payload
                .get(..2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]));
            self.diagnostic.emit("lan.close.frame", json!({"side":side,"code":code,"reasonBytes":frame.payload.len().saturating_sub(2)}));
        }
    }
    fn failure(&self, side: &str, error: &WebSocketError) {
        use WebSocketError::*;
        let cause = match error {
            InvalidFragment => "invalidFragment",
            InvalidContinuationFrame => "invalidContinuation",
            ReservedBitsNotZero => "reservedBits",
            ControlFrameFragmented => "fragmentedControl",
            PingFrameTooLarge => "oversizedPing",
            FrameTooLarge => "oversizedFrame",
            CompressionNotSupported => "compressionNotNegotiated",
            InvalidUTF8 => "invalidUtf8",
            InvalidCloseFrame => "invalidCloseFrame",
            InvalidCloseCode => "invalidCloseCode",
            InvalidOpCode(_) => "invalidOpcode",
            ConnectionClosed => "connectionClosed",
            IoError(_) => "io",
            _ => "other",
        };
        self.diagnostic.emit(
            "lan.stream.ended",
            json!({"side":side,"cause":cause,
            "osErrorCode":if let IoError(error) = error { error.raw_os_error() } else { None }}),
        );
    }
}
impl Stream for ObservedSocket {
    type Item = FrameView;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<FrameView>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        match this.socket.poll_next_frame(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(frame)) => {
                this.close_frame("received", &frame);
                Poll::Ready(Some(frame))
            }
            Poll::Ready(Err(error)) => {
                this.failure("read", &error);
                this.ended = true;
                Poll::Ready(None)
            }
        }
    }
}
impl Sink<FrameView> for ObservedSocket {
    type Error = WebSocketError;
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.socket).poll_ready(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.failure("writeReady", error);
        }
        result
    }
    fn start_send(self: Pin<&mut Self>, frame: FrameView) -> Result<(), Self::Error> {
        let this = self.get_mut();
        this.close_frame("sent", &frame);
        let result = Pin::new(&mut this.socket).start_send(frame);
        if let Err(error) = &result {
            this.failure("write", error);
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.socket).poll_flush(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.failure("flush", error);
        }
        result
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.get_mut().socket).poll_close(cx)
    }
}
