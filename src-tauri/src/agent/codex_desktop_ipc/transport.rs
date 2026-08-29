use super::protocol::DesktopIpcError;
use serde_json::Value;
use std::env;
use std::io::{Read, Write};
use std::path::PathBuf;

// The current Desktop router enforces the same cap. Keep this bounded before allocating.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

pub fn resolve_socket_path() -> Result<PathBuf, DesktopIpcError> {
    if let Some(path) = env::var_os("CODEX_IPC_SOCKET").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(codex_home) = env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(codex_home).join("ipc").join("ipc.sock"));
    }
    dirs::home_dir()
        .map(|home| home.join(".codex").join("ipc").join("ipc.sock"))
        .ok_or_else(|| DesktopIpcError::SocketPath("home directory could not be resolved".to_string()))
}

pub fn read_frame(reader: &mut impl Read) -> Result<Value, DesktopIpcError> {
    let mut header = [0_u8; 4];
    reader
        .read_exact(&mut header)
        .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 {
        return Err(DesktopIpcError::Protocol(
            "received an empty IPC frame".to_string(),
        ));
    }
    if length > MAX_FRAME_BYTES {
        return Err(DesktopIpcError::Protocol(format!(
            "IPC frame length {length} exceeds limit {MAX_FRAME_BYTES}"
        )));
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| DesktopIpcError::Io(error.to_string()))?;
    serde_json::from_slice(&body)
        .map_err(|error| DesktopIpcError::Protocol(format!("invalid IPC JSON: {error}")))
}

pub fn write_frame(writer: &mut impl Write, message: &Value) -> Result<(), DesktopIpcError> {
    let body = serde_json::to_vec(message)
        .map_err(|error| DesktopIpcError::Protocol(format!("failed to encode IPC JSON: {error}")))?;
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(DesktopIpcError::Protocol(format!(
            "encoded IPC frame length {} is outside 1..={MAX_FRAME_BYTES}",
            body.len()
        )));
    }
    let length = u32::try_from(body.len())
        .map_err(|_| DesktopIpcError::Protocol("IPC frame length exceeds u32".to_string()))?;
    writer
        .write_all(&length.to_le_bytes())
        .and_then(|_| writer.write_all(&body))
        .and_then(|_| writer.flush())
        .map_err(|error| DesktopIpcError::Io(error.to_string()))
}

#[cfg(unix)]
pub fn connect_validated_socket() -> Result<std::os::unix::net::UnixStream, DesktopIpcError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let path = resolve_socket_path()?;
    let parent = path.parent().ok_or_else(|| {
        DesktopIpcError::UnsafeSocket(format!("{} has no parent directory", path.display()))
    })?;
    let parent_metadata = std::fs::symlink_metadata(parent).map_err(|error| {
        DesktopIpcError::UnsafeSocket(format!("{}: {error}", parent.display()))
    })?;
    if !parent_metadata.file_type().is_dir() {
        return Err(DesktopIpcError::UnsafeSocket(format!(
            "{} is not a directory",
            parent.display()
        )));
    }
    let effective_uid = unsafe { libc::geteuid() };
    let parent_mode = parent_metadata.permissions().mode() & 0o777;
    if parent_mode != 0o700 || parent_metadata.uid() != effective_uid {
        return Err(DesktopIpcError::UnsafeSocket(format!(
            "{} must be owned by uid {effective_uid} with mode 0700; found uid {} mode {parent_mode:04o}",
            parent.display(),
            parent_metadata.uid()
        )));
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        DesktopIpcError::SocketPath(format!("{}: {error}", path.display()))
    })?;
    if !metadata.file_type().is_socket() {
        return Err(DesktopIpcError::UnsafeSocket(format!(
            "{} is not a Unix socket",
            path.display()
        )));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(DesktopIpcError::UnsafeSocket(format!(
            "{} has mode {mode:04o}; expected 0600",
            path.display()
        )));
    }
    if metadata.uid() != effective_uid {
        return Err(DesktopIpcError::UnsafeSocket(format!(
            "{} is owned by uid {}, current uid is {effective_uid}",
            path.display(),
            metadata.uid()
        )));
    }
    std::os::unix::net::UnixStream::connect(&path).map_err(|error| {
        DesktopIpcError::Io(format!("failed to connect {}: {error}", path.display()))
    })
}

#[cfg(not(unix))]
pub fn unsupported_platform_error() -> DesktopIpcError {
    DesktopIpcError::Unsupported(
        "the current Codex Desktop private transport is a Unix socket".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn frame_codec_round_trips_little_endian_json() {
        let message = json!({ "type": "broadcast", "value": "你好" });
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &message).unwrap();
        let declared = u32::from_le_bytes(encoded[..4].try_into().unwrap()) as usize;
        assert_eq!(declared, encoded.len() - 4);
        assert_eq!(read_frame(&mut Cursor::new(encoded)).unwrap(), message);
    }

    #[test]
    fn frame_codec_rejects_oversized_and_truncated_frames() {
        let oversized = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_le_bytes();
        assert!(matches!(
            read_frame(&mut Cursor::new(oversized)),
            Err(DesktopIpcError::Protocol(message)) if message.contains("exceeds limit")
        ));

        let truncated = [4_u8, 0, 0, 0, b'{', b'}'];
        assert!(matches!(
            read_frame(&mut Cursor::new(truncated)),
            Err(DesktopIpcError::Io(_))
        ));

        let invalid_json = [2_u8, 0, 0, 0, b'{', b'}'];
        assert!(matches!(
            read_frame(&mut Cursor::new(invalid_json)),
            Ok(Value::Object(_))
        ));
        let invalid_json = [1_u8, 0, 0, 0, b'{'];
        assert!(matches!(
            read_frame(&mut Cursor::new(invalid_json)),
            Err(DesktopIpcError::Protocol(message)) if message.contains("invalid IPC JSON")
        ));
    }
}
