//! Native JSON Lines framing. A rejected, fully drained line does not end the stream.
use super::{CodexAppServerError, JsonRpcId, JsonRpcReader, MAX_APP_SERVER_STDERR_LINE_BYTES};
use serde::de::{DeserializeSeed, MapAccess, Visitor};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read};

pub(super) struct JsonLineReader<R> {
    reader: BufReader<R>,
}

impl<R: Read> JsonLineReader<R> {
    pub(super) fn new(reader: R) -> Self { Self { reader: BufReader::new(reader) } }
}

impl<R: Read + Send + 'static> JsonRpcReader for JsonLineReader<R> {
    fn read_message(&mut self) -> Result<Option<Value>, CodexAppServerError> {
        if self.reader.fill_buf().map_err(io_error)?.is_empty() { return Ok(None); }
        let mut line = PhysicalLine::new(&mut self.reader);
        let mut envelope = Envelope::default();
        // A native turn has no size budget. Decode directly from the physical
        // line so a large response does not need a second raw byte buffer.
        let decoded = {
            let mut decoder = serde_json::Deserializer::from_reader(&mut line);
            (&mut envelope).deserialize(&mut decoder).and_then(|message| {
                decoder.end()?;
                Ok(message)
            })
        };
        io::copy(&mut line, &mut io::sink()).map_err(io_error)?;
        match decoded {
            Ok(message) => Ok(Some(message)),
            Err(error) if error.is_io() => Err(CodexAppServerError::Io(error.to_string())),
            Err(error) => Err(envelope.reject(CodexAppServerError::Protocol(format!("invalid JSON: {error}")))),
        }
    }
}

// Expose exactly one physical line to a streaming JSON decoder. Rejected JSON
// can be drained to the newline without consuming any of the following frame.
struct PhysicalLine<'a, R> {
    reader: &'a mut R,
    done: bool,
    bytes: usize,
}

impl<'a, R> PhysicalLine<'a, R> {
    fn new(reader: &'a mut R) -> Self { Self { reader, done: false, bytes: 0 } }
}

impl<R: BufRead> Read for PhysicalLine<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.done || output.is_empty() { return Ok(0); }
        let available = self.reader.fill_buf()?;
        let available = &available[..available.len().min(output.len())];
        if available.is_empty() { self.done = true; return Ok(0); }
        let count = available.iter().position(|byte| *byte == b'\n')
            .map(|index| index + 1).unwrap_or(available.len());
        self.done = available[count - 1] == b'\n';
        output[..count].copy_from_slice(&available[..count]);
        self.reader.consume(count);
        self.bytes = self.bytes.saturating_add(count);
        Ok(count)
    }
}

#[derive(Default)]
struct Envelope {
    request_id: Option<JsonRpcId>,
    method: Option<String>,
    response: bool,
}

impl Envelope {
    fn reject(self, error: CodexAppServerError) -> CodexAppServerError {
        CodexAppServerError::RejectedMessage {
            request_id: if self.response || self.method.is_some() { self.request_id } else { None },
            method: self.method, error: Box::new(error),
        }
    }
}

impl<'de> DeserializeSeed<'de> for &mut Envelope {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for &mut Envelope {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON-RPC envelope")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
        let mut fields = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if fields.contains_key(&key) {
                if key == "id" { self.request_id = None; }
                return Err(serde::de::Error::custom("duplicate JSON-RPC field"));
            }
            match key.as_str() {
                "method" | "params" => { self.method.get_or_insert_with(String::new); }
                "result" | "error" => self.response = true,
                _ => {}
            }
            let value: Value = map.next_value()?;
            match key.as_str() {
                "id" => self.request_id = serde_json::from_value(value.clone()).ok(),
                "method" => self.method = Some(value.as_str().unwrap_or("").to_owned()),
                _ => {}
            }
            fields.insert(key, value);
        }
        Ok(Value::Object(fields))
    }

}

pub(super) fn drain_stderr(stderr: impl Read) -> Result<(), CodexAppServerError> {
    let mut reader = BufReader::new(stderr);
    loop {
        let mut line = PhysicalLine::new(&mut reader);
        let mut captured = Vec::new();
        (&mut line).take(MAX_APP_SERVER_STDERR_LINE_BYTES as u64).read_to_end(&mut captured).map_err(io_error)?;
        if captured.is_empty() { return Ok(()); }
        io::copy(&mut line, &mut io::sink()).map_err(io_error)?;
        eprintln!("Codex App Server: {}", String::from_utf8_lossy(&captured).trim_end());
        if line.bytes > captured.len() {
            eprintln!("Codex App Server stderr line truncated: bytes={} retained={}", line.bytes, captured.len());
        }
    }
}

fn io_error(error: io::Error) -> CodexAppServerError { CodexAppServerError::Io(error.to_string()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_response_larger_than_sixteen_mib_is_read_without_a_turn_budget() {
        let text = "x".repeat(16 * 1024 * 1024 + 1);
        let large = format!("{{\"result\":{{\"text\":\"{text}\"}},\"id\":41}}\n");
        let mut reader = JsonLineReader::new(std::io::Cursor::new(format!("{large}{{\"id\":42,\"result\":{{}}}}\n")));
        assert_eq!(reader.read_message().unwrap().unwrap()["result"]["text"].as_str().unwrap().len(), text.len());
        assert_eq!(reader.read_message().unwrap().unwrap()["id"], 42);
    }

    #[test]
    fn malformed_frame_preserves_known_request_id_and_continues_at_next_line() {
        let mut reader = JsonLineReader::new(std::io::Cursor::new(b"{\"id\":41,\"result\":!}\n{\"id\":42,\"result\":{}}\n"));
        assert!(matches!(reader.read_message().unwrap_err(), CodexAppServerError::RejectedMessage {
            request_id: Some(JsonRpcId::Number(41)), ..
        }));
        assert_eq!(reader.read_message().unwrap().unwrap()["id"], 42);
    }

    #[test]
    fn malformed_server_request_keeps_its_own_id_namespace() {
        let data = "{\"id\":41,\"method\":\"approval\",\"params\":!}\n";
        let mut reader = JsonLineReader::new(std::io::Cursor::new(data));
        assert!(matches!(reader.read_message().unwrap_err(), CodexAppServerError::RejectedMessage {
            request_id: Some(JsonRpcId::Number(41)), method: Some(_), ..
        }));
    }
}
