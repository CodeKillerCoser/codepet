//! Blocking STDIO bridge. Its output completion acknowledges the real writer's drain.
use super::StdioServerError;
use std::io::{Read, Write};

pub(super) fn bridge<R: Read + Send + 'static, W: Write + Send + 'static>(
    mut reader: R, mut writer: W,
) -> Result<(tokio::io::DuplexStream, tokio::sync::oneshot::Receiver<Result<(), String>>), StdioServerError> {
    let (io, bridge) = tokio::io::duplex(64 * 1024);
    let (mut read, mut write) = tokio::io::split(bridge);
    let handle = tokio::runtime::Handle::current();
    let input_handle = handle.clone();
    std::thread::Builder::new().name("provider-mux-input".into()).spawn(move || {
        let mut buf = [0; 16 * 1024];
        loop {
            let n = match reader.read(&mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
            if input_handle.block_on(tokio::io::AsyncWriteExt::write_all(&mut write, &buf[..n])).is_err() { break; }
        }
        // Dropping a generic split write half does not close the duplex while
        // the output pump still owns its read half. Propagate stdin EOF explicitly.
        let _ = input_handle.block_on(tokio::io::AsyncWriteExt::shutdown(&mut write));
    }).map_err(|e| StdioServerError::new(e.to_string()))?;
    let (output_done, output_flushed) = tokio::sync::oneshot::channel();
    std::thread::Builder::new().name("provider-mux-output".into()).spawn(move || {
        let mut buf = [0; 16 * 1024];
        let result = loop {
            let n = match handle.block_on(tokio::io::AsyncReadExt::read(&mut read, &mut buf)) {
                Ok(0) => break Ok(()), Ok(n) => n, Err(e) => break Err(e.to_string()),
            };
            if let Err(e) = writer.write_all(&buf[..n]).and_then(|_| writer.flush()) { break Err(e.to_string()); }
        };
        drop(writer);
        let _ = output_done.send(result);
    }).map_err(|e| StdioServerError::new(e.to_string()))?;
    Ok((io, output_flushed))
}
