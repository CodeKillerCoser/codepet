use codepet_provider_claude::ClaudeProvider;
use codepet_provider_sdk::{
    dispatch, JsonLineCodec, JsonRpcInboundRequest, ProtocolEvent, ProtocolServer,
    ProviderShutdownRequest, ProviderWireMessage,
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

    loop {
        let message = match codec.read_message(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => {
                let message = error.error.message.clone();
                write_message(
                    &codec,
                    &writer,
                    ProviderWireMessage::Response(error.into_response()),
                )?;
                return Err(message);
            }
        };
        let request = match message {
            ProviderWireMessage::Request(JsonRpcInboundRequest::Typed(request)) => request,
            ProviderWireMessage::Request(JsonRpcInboundRequest::Rejected(error)) => {
                write_message(
                    &codec,
                    &writer,
                    ProviderWireMessage::Response(error.into_response()),
                )?;
                continue;
            }
            ProviderWireMessage::Response(_)
            | ProviderWireMessage::Notification(_)
            | ProviderWireMessage::Event(_) => continue,
        };
        let response = dispatch(provider.as_ref(), request).await;
        write_message(
            &codec,
            &writer,
            ProviderWireMessage::Response(response),
        )?;
        if provider.is_shutdown() {
            break;
        }
    }

    if !provider.is_shutdown() {
        let _ = ProtocolServer::provider_shutdown(
            provider.as_ref(),
            ProviderShutdownRequest {},
        )
        .await;
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
