use codepet_provider_codex::CodexProvider;
use codepet_provider_sdk::{
    dispatch, JsonLineCodec, JsonRpcInboundRequest, ProtocolEvent, ProviderShutdownRequest,
    ProviderWireMessage, ProtocolServer, MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES,
};
use std::io::{BufReader, BufWriter, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const MAX_CONCURRENT_HOST_REQUESTS: usize = 16;
const MAX_PENDING_HOST_MESSAGES: usize = 32;
const DISPATCH_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

struct StdioEventSink {
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
}

impl codepet_provider_codex::ProviderEventSink for StdioEventSink {
    fn publish(&self, event: ProtocolEvent) -> Result<(), codepet_provider_sdk::ProtocolError> {
        let mut writer = lock(&self.writer);
        self.codec
            .write_message(&mut *writer, &ProviderWireMessage::Event(event))?;
        writer.flush().map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "json_line_flush_failed".to_string(),
            message: format!("flush Provider event: {error}"),
            retryable: true,
            details: None,
        })
    }
}

enum ReaderTerminal {
    Eof,
    Fatal {
        response: ProviderWireMessage,
        message: String,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Codex Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let codec = JsonLineCodec::new(MAX_CONVERSATION_HISTORY_JSON_LINE_BYTES)
        .map_err(|error| error.message)?;
    let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let events = Arc::new(StdioEventSink {
        codec,
        writer: writer.clone(),
    });
    let provider = Arc::new(CodexProvider::new(events));
    let (message_sender, message_receiver) = mpsc::channel(MAX_PENDING_HOST_MESSAGES);
    let (terminal_sender, terminal_receiver) = mpsc::channel(1);
    std::thread::spawn(move || {
        read_host_messages(codec, message_sender, terminal_sender);
    });

    run_service_loop(
        provider,
        codec,
        writer,
        message_receiver,
        terminal_receiver,
    )
    .await
}

fn read_host_messages(
    codec: JsonLineCodec,
    messages: mpsc::Sender<ProviderWireMessage>,
    terminal: mpsc::Sender<ReaderTerminal>,
) {
    let mut reader = BufReader::new(std::io::stdin());
    loop {
        match codec.read_message(&mut reader) {
            Ok(Some(message)) => {
                if messages.blocking_send(message).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = terminal.blocking_send(ReaderTerminal::Eof);
                return;
            }
            Err(error) => {
                let message = error.error.message.clone();
                let _ = terminal.blocking_send(ReaderTerminal::Fatal {
                    response: ProviderWireMessage::Response(error.into_response()),
                    message,
                });
                return;
            }
        }
    }
}

async fn run_service_loop(
    provider: Arc<CodexProvider>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    mut messages: mpsc::Receiver<ProviderWireMessage>,
    mut terminal: mpsc::Receiver<ReaderTerminal>,
) -> Result<(), String> {
    let mut tasks = JoinSet::new();
    let mut service_result = Ok(());
    let mut fatal_response = None;

    loop {
        if tasks.len() >= MAX_CONCURRENT_HOST_REQUESTS {
            tokio::select! {
                biased;
                terminal_message = terminal.recv() => {
                    match terminal_message.unwrap_or(ReaderTerminal::Eof) {
                        ReaderTerminal::Eof => {}
                        ReaderTerminal::Fatal { response, message } => {
                            fatal_response = Some((response, message));
                        }
                    }
                    break;
                }
                completed = tasks.join_next() => {
                    match completed.expect("dispatch set is not empty") {
                        Ok(Ok(true)) => break,
                        Ok(Ok(false)) => {}
                        Ok(Err(error)) => {
                            service_result = Err(error);
                            break;
                        }
                        Err(error) => {
                            service_result = Err(format!("Provider dispatch task failed: {error}"));
                            break;
                        }
                    }
                }
            }
            continue;
        }

        tokio::select! {
            biased;
            terminal_message = terminal.recv() => {
                match terminal_message.unwrap_or(ReaderTerminal::Eof) {
                    ReaderTerminal::Eof => {}
                    ReaderTerminal::Fatal { response, message } => {
                        fatal_response = Some((response, message));
                    }
                }
                break;
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                match completed.expect("dispatch set is not empty") {
                    Ok(Ok(true)) => break,
                    Ok(Ok(false)) => {}
                    Ok(Err(error)) => {
                        service_result = Err(error);
                        break;
                    }
                    Err(error) => {
                        service_result = Err(format!("Provider dispatch task failed: {error}"));
                        break;
                    }
                }
            }
            message = messages.recv() => {
                let Some(message) = message else {
                    break;
                };
                let task_provider = provider.clone();
                let task_writer = writer.clone();
                tasks.spawn(async move {
                    dispatch_host_message(task_provider, codec, task_writer, message).await
                });
            }
        }
    }

    let cleanup_result = shutdown_and_drain(provider.as_ref(), &mut tasks).await;
    if let Some((response, message)) = fatal_response {
        if let Err(error) = cleanup_result {
            eprintln!("Codex Provider cleanup after invalid Host frame failed: {error}");
        }
        write_message(&codec, &writer, response)?;
        return Err(format!("invalid Host frame: {message}"));
    }
    match service_result {
        Err(error) => {
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("Codex Provider cleanup also failed: {cleanup_error}");
            }
            Err(error)
        }
        Ok(()) => cleanup_result,
    }
}

async fn dispatch_host_message(
    provider: Arc<CodexProvider>,
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    message: ProviderWireMessage,
) -> Result<bool, String> {
    let response = match message {
        ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => {
            dispatch(provider.as_ref(), request).await
        }
        ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(rejection)) => {
            rejection.into_response()
        }
        ProviderWireMessage::Response(_)
        | ProviderWireMessage::Notification(_)
        | ProviderWireMessage::Event(_) => {
            eprintln!("Codex Provider ignored a non-request Host message");
            return Ok(false);
        }
    };
    write_message(&codec, &writer, ProviderWireMessage::Response(response))?;
    Ok(provider.is_shutdown())
}

async fn shutdown_and_drain(
    provider: &CodexProvider,
    tasks: &mut JoinSet<Result<bool, String>>,
) -> Result<(), String> {
    let mut first_error = None;
    if !provider.is_shutdown() {
        if let Err(error) = ProtocolServer::provider_shutdown(provider, ProviderShutdownRequest {}).await {
            first_error = Some(error.message);
        }
    }
    let drained = tokio::time::timeout(DISPATCH_DRAIN_TIMEOUT, async {
        while let Some(completed) = tasks.join_next().await {
            match completed {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(format!("Provider dispatch task failed: {error}"));
                    }
                }
            }
        }
    })
    .await
    .is_ok();
    if !drained {
        tasks.abort_all();
        let _ = tokio::time::timeout(DISPATCH_DRAIN_TIMEOUT, async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
        if first_error.is_none() {
            first_error = Some("timed out draining Provider dispatch tasks".to_string());
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn write_message(
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    message: ProviderWireMessage,
) -> Result<(), String> {
    let mut writer = lock(writer);
    codec
        .write_message(&mut *writer, &message)
        .map_err(|error| error.message)?;
    writer.flush().map_err(|error| error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
