use codepet_provider_claude::ClaudeProvider;
use codepet_provider_sdk::{
    dispatch, JsonLineCodec, JsonRpcInboundRequest, ProtocolEvent, ProviderWireMessage,
};
use std::io::{BufReader, BufWriter, Write};
use std::sync::{Arc, Mutex, MutexGuard};

struct StdioEventSink {
    codec: JsonLineCodec,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
}

impl codepet_provider_claude::ProviderEventSink for StdioEventSink {
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

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Claude Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let codec = JsonLineCodec::default();
    let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let events = Arc::new(StdioEventSink {
        codec,
        writer: writer.clone(),
    });
    let provider = Arc::new(ClaudeProvider::new(events));
    let mut reader = BufReader::new(std::io::stdin());

    let service_result = run_service_loop(&provider, &codec, &writer, &mut reader).await;
    let cleanup_result = provider
        .reap_active_processes()
        .map_err(|error| format!("reap Claude Provider processes: {}", error.message));
    match service_result {
        Err(error) => {
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("Claude Provider cleanup also failed: {cleanup_error}");
            }
            Err(error)
        }
        Ok(()) => cleanup_result,
    }
}

async fn run_service_loop(
    provider: &ClaudeProvider,
    codec: &JsonLineCodec,
    writer: &Arc<Mutex<BufWriter<std::io::Stdout>>>,
    reader: &mut BufReader<std::io::Stdin>,
) -> Result<(), String> {
    loop {
        let message = match codec.read_message(&mut *reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => return Err(error.error.message),
        };
        let request = match message {
            ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => request,
            ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(error)) => {
                write_message(
                    codec,
                    writer,
                    ProviderWireMessage::Response(error.into_response()),
                )?;
                continue;
            }
            ProviderWireMessage::Response(_)
            | ProviderWireMessage::Notification(_)
            | ProviderWireMessage::Event(_) => continue,
        };
        let response = dispatch(provider, request).await;
        write_message(
            codec,
            writer,
            ProviderWireMessage::Response(response),
        )?;
        if provider.is_shutdown() {
            break;
        }
    }
    Ok(())
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
