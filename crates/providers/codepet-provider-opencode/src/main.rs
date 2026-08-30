use codepet_provider_opencode::OpenCodeProvider;
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

impl codepet_provider_opencode::ProviderEventSink for StdioEventSink {
    fn publish(&self, event: ProtocolEvent) -> Result<(), codepet_provider_sdk::ProtocolError> {
        let mut writer = lock(&self.writer);
        self.codec
            .write_message(&mut *writer, &ProviderWireMessage::Event(event))?;
        writer.flush().map_err(|error| codepet_provider_sdk::ProtocolError {
            code: "json_line_flush_failed".to_string(),
            message: format!("flush OpenCode Provider event: {error}"),
            retryable: true,
            details: None,
        })
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("OpenCode Provider stopped with an error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let codec = JsonLineCodec::default();
    let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let provider = Arc::new(OpenCodeProvider::new(Arc::new(StdioEventSink {
        codec,
        writer: writer.clone(),
    })));
    let mut reader = BufReader::new(std::io::stdin());

    let loop_result = async {
        loop {
            let message = match codec.read_message(&mut reader) {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    let detail = error.error.message.clone();
                    write_message(
                        &codec,
                        &writer,
                        ProviderWireMessage::Response(error.into_response()),
                    )?;
                    return Err(format!("invalid Host frame: {detail}"));
                }
            };
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
                    eprintln!("OpenCode Provider ignored a non-request Host message");
                    continue;
                }
            };
            write_message(
                &codec,
                &writer,
                ProviderWireMessage::Response(response),
            )?;
            if provider.is_shutdown() {
                break;
            }
        }
        Ok(())
    }
    .await;

    let cleanup_result = ProtocolServer::provider_shutdown(
        provider.as_ref(),
        ProviderShutdownRequest {},
    )
    .await
    .map(|_| ())
    .map_err(|error| format!("OpenCode Provider cleanup failed: {}", error.message));
    match (loop_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}")),
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
