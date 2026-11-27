use super::*;

pub(super) const OUTPUT_CAPACITY: usize = 1024 * 1024;
const READ_CHUNK: usize = 32 * 1024;

#[derive(Default)]
pub(super) struct Output {
    bytes: VecDeque<u8>,
    start: u64,
    eof: bool,
    waited: bool,
    failed: bool,
}

impl Output {
    pub(super) fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
        let excess = self.bytes.len().saturating_sub(OUTPUT_CAPACITY);
        self.bytes.drain(..excess);
        self.start += excess as u64;
    }

    pub(super) fn read(&self, offset: u64) -> Result<gateway::TerminalReadResponse, Error> {
        if offset < self.start {
            return Err(error(
                "terminal_output_expired",
                "终端输出已超过保留范围，请新建终端",
                false,
            ));
        }
        let index = offset
            .checked_sub(self.start)
            .filter(|n| *n <= self.bytes.len() as u64)
            .ok_or_else(|| {
                error(
                    "terminal_invalid_offset",
                    "Invalid terminal byte offset",
                    false,
                )
            })? as usize;
        let bytes: Vec<_> = self
            .bytes
            .iter()
            .skip(index)
            .take(READ_CHUNK)
            .copied()
            .collect();
        if bytes.is_empty() && self.failed {
            return Err(error(
                "terminal_read_failed",
                "PTY output could not be read",
                false,
            ));
        }
        let next_offset = offset + bytes.len() as u64;
        Ok(gateway::TerminalReadResponse {
            data: BASE64.encode(bytes),
            next_offset,
            exited: self.eof && self.waited && next_offset == self.start + self.bytes.len() as u64,
        })
    }

    pub(super) fn finished(&self) -> bool {
        self.eof && self.waited
    }

    pub(super) fn start_reader(
        output: Arc<Mutex<Self>>,
        mut reader: Box<dyn Read + Send>,
    ) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("codepet-pty-output".into())
            .spawn(move || {
                let mut bytes = [0; 8192];
                loop {
                    match reader.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(count) => {
                            if let Ok(mut out) = output.lock() {
                                out.append(&bytes[..count]);
                            } else {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        // Unix PTYs commonly report EIO when the slave closes.
                        Err(e) if platform::is_eof_error(&e) => break,
                        Err(_) => {
                            if let Ok(mut out) = output.lock() {
                                out.failed = true;
                            }
                            break;
                        }
                    }
                }
                if let Ok(mut out) = output.lock() {
                    out.eof = true;
                }
            })?;
        Ok(())
    }

    pub(super) fn start_waiter(output: Arc<Mutex<Self>>, child: Child) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("codepet-pty-wait".into())
            .spawn(move || loop {
                let done = match child.lock() {
                    Ok(mut guard) => match guard.as_mut().map(|child| child.try_wait()) {
                        Some(Ok(None)) => false,
                        result => {
                            let failed = matches!(result, Some(Err(_)));
                            guard.take(); // Reaped PID must never be signalled by a later close.
                            if let Ok(mut out) = output.lock() {
                                out.waited = true;
                                out.failed |= failed;
                            }
                            true
                        }
                    },
                    Err(_) => true,
                };
                if done {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            })?;
        Ok(())
    }
}
