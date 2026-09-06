use crate::generated::ProtocolServer;
use crate::transport::{TRANSPORT_ENV, MUX_PROFILE};
use super::{ProviderEventSink, StdioServerError, StdioServerOptions};
use std::{io::{BufReader, BufWriter, Read, Write}, sync::Arc};

pub async fn serve_stdio<P, F>(
    options: StdioServerOptions,
    factory: F,
) -> Result<(), StdioServerError>
where
    P: ProtocolServer + 'static,
    F: FnOnce(Arc<dyn ProviderEventSink>) -> P,
{
    serve_stdio_with_io(
        BufReader::new(std::io::stdin()),
        BufWriter::new(std::io::stdout()),
        options,
        factory,
    )
    .await
}

/// Runs the mux stdio transport runtime over caller-provided I/O.
///
/// This is public so Provider authors can build process-level conformance tests without
/// reimplementing the transport. Production plugins should normally call [`serve_stdio`].
pub async fn serve_stdio_with_io<R, W, P, F>(
    reader: R,
    writer: W,
    options: StdioServerOptions,
    factory: F,
) -> Result<(), StdioServerError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    P: ProtocolServer + 'static,
    F: FnOnce(Arc<dyn ProviderEventSink>) -> P,
{
    match std::env::var(TRANSPORT_ENV) {
        Ok(value) if value == MUX_PROFILE => {},
        Err(std::env::VarError::NotPresent) => {},
        Ok(value) => return Err(StdioServerError::new(format!("unsupported Provider transport: {value}"))),
        Err(error) => return Err(StdioServerError::new(format!("invalid Provider transport: {error}"))),
    }
    super::mux::serve_mux_with_io(reader, writer, options, factory).await
}
