//! Synchronous test handles backed by the real negotiated mux transport.
//! Only this fixture bridges sync tests to Tokio; providers never depend on Host.
use codepet_provider_sdk::{default_transport_limits, ProviderMux, ProviderWireMessage};
use serde_json::Value;
use std::{io, pin::Pin, process::{ChildStdin, ChildStdout}, sync::{mpsc, Arc, Mutex}, task::{Context, Poll, Waker}, time::Duration};
use tokio::io::{AsyncRead, ReadBuf};

type ReadState = Arc<Mutex<(Option<Pin<Box<dyn AsyncRead + Send>>>, bool, Option<Waker>)>>;
struct ControlledRead(ReadState);
impl AsyncRead for ControlledRead {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let mut state = self.0.lock().unwrap();
        state.2 = Some(cx.waker().clone());
        if state.1 { return Poll::Pending; }
        match state.0.as_mut() { Some(reader) => reader.as_mut().poll_read(cx, buf), None => Poll::Ready(Ok(())) }
    }
}

pub struct Writer {
    commands: tokio::sync::mpsc::UnboundedSender<ProviderWireMessage>,
    // Retain a pipe owner for physical fault injection even if the mux reader stops.
    raw: std::fs::File,
}
impl Writer {
    pub fn send(&mut self, value: Value) {
        let message = codepet_provider_sdk::decode_wire_message(&serde_json::to_vec(&value).unwrap()).unwrap();
        self.commands.send(message).expect("mux command receiver closed");
    }
    #[allow(dead_code)]
    pub fn corrupt_transport(&mut self, oversized: bool) {
        use io::Write;
        let mut header = [0u8; 12];
        if oversized { header[8..].copy_from_slice(&u32::MAX.to_be_bytes()); }
        else { header[0] = 255; }
        self.raw.write_all(&header).unwrap();
        self.raw.flush().unwrap();
    }
}

pub struct Reader {
    pub messages: mpsc::Receiver<Value>,
    state: ReadState,
}
impl Reader {
    #[allow(dead_code)]
    pub fn recv(&self) -> Option<Value> {
        match self.messages.recv_timeout(Duration::from_secs(10)) {
            Ok(value) => Some(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
            Err(error) => panic!("mux receive: {error}"),
        }
    }
    #[allow(dead_code)]
    pub fn pause(&self) { self.state.lock().unwrap().1 = true; }
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.0.take();
        // Close the physical read end without shutting down the independent write end.
        state.1 = true;
        if let Some(waker) = state.2.take() { waker.wake(); }
    }
}
impl Drop for Reader { fn drop(&mut self) { self.close(); } }

pub fn connect(stdin: ChildStdin, stdout: ChildStdout) -> (Writer, Reader) {
    connect_reader(stdin, move || Box::pin(tokio::process::ChildStdout::from_std(stdout).unwrap()))
}
#[cfg(unix)]
#[allow(dead_code)]
pub fn connect_socket(stdin: ChildStdin, stdout: std::os::unix::net::UnixStream) -> (Writer, Reader) {
    connect_reader(stdin, move || {
        stdout.set_nonblocking(true).unwrap();
        Box::pin(tokio::net::UnixStream::from_std(stdout).unwrap())
    })
}
fn connect_reader<F>(stdin: ChildStdin, reader: F) -> (Writer, Reader)
where F: FnOnce() -> Pin<Box<dyn AsyncRead + Send>> + Send + 'static {
    #[cfg(unix)]
    let raw = { use std::os::fd::AsFd; std::fs::File::from(stdin.as_fd().try_clone_to_owned().unwrap()) };
    #[cfg(windows)]
    let raw = { use std::os::windows::io::AsHandle; std::fs::File::from(stdin.as_handle().try_clone_to_owned().unwrap()) };
    let (commands, mut requests) = tokio::sync::mpsc::unbounded_channel::<ProviderWireMessage>();
    let (sender, messages) = mpsc::channel();
    let (ready, connected) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
            let state = Arc::new(Mutex::new((Some(reader()), false, None)));
            let write = tokio::process::ChildStdin::from_std(stdin).unwrap();
            let (peer, mut events, mut driver) = ProviderMux::connect(ControlledRead(state.clone()), write, true, default_transport_limits()).await.unwrap();
            ready.send(state).unwrap();
            let mut work = tokio::task::JoinSet::new();
            let mut input_closed = false;
            loop {
                tokio::select! {
                    _ = &mut driver => break,
                    request = requests.recv() => {
                        let Some(request) = request else { input_closed = true; break; };
                        let peer = peer.clone(); let sender = sender.clone();
                        work.spawn(async move {
                            let response = peer.exchange(request).await;
                            match response {
                                Ok(response) => { let _ = sender.send(wire_value(response.message)); }
                                Err(error) => eprintln!("Test mux exchange ended: {error:?}"),
                            }
                        });
                    }
                    incoming = events.recv() => {
                        let Some(mut incoming) = incoming else { break; };
                        // Keep event reception and ACK ordered, like the real Host consumer.
                        if let Ok(received) = incoming.receive().await {
                            if sender.send(wire_value(received.message)).is_err() { break; }
                            let _ = incoming.acknowledge().await;
                        }
                    }
                    _ = work.join_next(), if !work.is_empty() => {}
                }
            }
            if input_closed {
                peer.close();
                if !driver.is_finished() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), &mut driver).await;
                }
            }
            // GOAWAY/EOF can arrive before the task consumes the buffered final response.
            if !input_closed {
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                while work.join_next().await.is_some() {}
            }).await;
            }
            work.abort_all();
            driver.abort();
        });
    });
    let state = connected.recv_timeout(Duration::from_secs(5)).expect("Provider mux handshake failed");
    (Writer { commands, raw }, Reader { messages, state })
}
fn wire_value(message: ProviderWireMessage) -> Value {
    match message {
        ProviderWireMessage::Response(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Notification(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Event(value) => serde_json::to_value(value).unwrap(),
        ProviderWireMessage::Request(_) => panic!("Provider sent an unexpected request"),
    }
}
