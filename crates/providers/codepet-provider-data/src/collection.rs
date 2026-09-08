//! Event-driven durable Hook intake. Thirty minutes is the bucket width, not a timer.
use crate::*;
use rusqlite::params;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

pub struct UsageSink {
    pub data: Arc<ProviderData>,
    pub downstream: Arc<dyn ProviderEventSink>,
    pub provider: &'static str,
}
impl ProviderEventSink for UsageSink {
    fn publish(&self, event: ProtocolEvent) -> Result<(), ProtocolError> {
        if self.provider != "codex" && self.data.configured() {
            if let ProtocolEvent::EventNotification { params, .. } = &event {
                let result = (|| {
                    let db = self.data.connection()?;
                    let raw = params
                        .payload
                        .get("codepet_observation")
                        .and_then(|v| v.get("raw"))
                        .cloned()
                        .unwrap_or_else(|| json!(params.payload));
                    if self.provider == "claude" {
                        let name = raw["hook_event_name"].as_str().unwrap_or("");
                        if !["Stop", "SessionEnd", "SubagentStop", "StopFailure"].contains(&name) {
                            return Ok(());
                        }
                        // Tool activity still reaches Pet, but does not enqueue usage work.
                        // SubagentStop can carry a separate transcript; both sources are idempotent.
                        for key in ["transcript_path", "agent_transcript_path"] {
                            if let Some(path) = raw[key]
                                .as_str()
                                .filter(|p| std::path::Path::new(p).is_absolute())
                            {
                                db.execute("INSERT INTO usage_sources(path,position,fingerprint) VALUES(?1,0,'') ON CONFLICT(path) DO UPDATE SET checked=0",[path]).map_err(db_error)?;
                            }
                        }
                        self.data.wake.notify_one();
                        return Ok(());
                    }
                    if self.provider == "opencode" && raw["type"] != "message.updated" {
                        return Ok(());
                    }
                    db.execute("INSERT OR IGNORE INTO usage_inbox(id,provider,payload,received) VALUES(?1,?2,?3,?4)",
                        params![params.event_id,self.provider,raw.to_string(),params.received_at]).map_err(db_error)?;
                    self.data.wake.notify_one();
                    Ok::<_, ProtocolError>(())
                })();
                if let Err(e) = result {
                    eprintln!("usage.intake failed: {}", e.message);
                }
            }
        }
        self.downstream.publish(event)
    }
}

impl ProviderData {
    pub fn start_collection(self: &Arc<Self>) -> Result<(), ProtocolError> {
        if !self.configured() {
            return Ok(());
        }
        let mut worker = self.worker.lock().map_err(db_error)?;
        if worker.is_some() {
            return Ok(());
        }
        let weak = Arc::downgrade(self);
        let wake = self.wake.clone();
        *worker = Some(tokio::spawn(async move {
            loop {
                let Some(data) = weak.upgrade() else { break };
                let result = tokio::task::spawn_blocking(move || data.collect()).await;
                if let Err(e) = result {
                    eprintln!("usage.collect task failed: {e}");
                } else if let Ok(Err(e)) = result {
                    eprintln!("usage.collect failed: {}", e.message);
                }
                // Retry a trailing transcript write after an end event. Periodic reconciliation
                // only checks registered sources; it never enumerates the complete history tree.
                if tokio::time::timeout(std::time::Duration::from_secs(300), wake.notified())
                    .await
                    .is_ok()
                {
                    for delay_ms in [0, 1000, 4000] {
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        let Some(data) = weak.upgrade() else { return };
                        if let Ok(Err(e)) =
                            tokio::task::spawn_blocking(move || data.collect()).await
                        {
                            eprintln!("usage.collect retry failed: {}", e.message);
                        }
                    }
                }
            }
        }));
        Ok(())
    }

    pub fn collect(&self) -> Result<(), ProtocolError> {
        let db = self.connection()?;
        let pending: Vec<(String, String, String)> = db
            .prepare("SELECT id,provider,payload FROM usage_inbox ORDER BY received,id LIMIT 1000")
            .map_err(db_error)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(db_error)?
            .collect::<Result<_, _>>()
            .map_err(db_error)?;
        for (id, provider, payload) in pending {
            let raw: Value = serde_json::from_str(&payload).map_err(db_error)?;
            if provider == "claude" {
                if let Some(path) = raw
                    .get("transcript_path")
                    .and_then(Value::as_str)
                    .filter(|p| std::path::Path::new(p).is_absolute())
                {
                    db.execute("INSERT INTO usage_sources(path,position,fingerprint) VALUES(?1,0,'') ON CONFLICT(path) DO NOTHING",[path]).map_err(db_error)?;
                }
            } else if provider == "opencode" {
                self.opencode_message(&raw)?;
            }
            db.execute("DELETE FROM usage_inbox WHERE id=?1", [id])
                .map_err(db_error)?;
        }
        // Bounded per pass: at most 32 files and 4 MiB/file. Resume positions survive restart.
        let sources:Vec<(String,u64,String)>=db.prepare("SELECT path,position,fingerprint FROM usage_sources ORDER BY checked,path LIMIT 32").map_err(db_error)?
            .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?.collect::<Result<_,_>>().map_err(db_error)?;
        for (path, position, fingerprint) in sources {
            if let Err(e) = self.claude_source(&path, position, &fingerprint) {
                eprintln!("usage.transcript failed: {}", e.message);
            }
            db.execute(
                "UPDATE usage_sources SET checked=?2 WHERE path=?1",
                params![path, chrono::Utc::now().timestamp_millis()],
            )
            .map_err(db_error)?;
        }
        eprintln!("usage.collect completed");
        Ok(())
    }

    fn claude_source(
        &self,
        path: &str,
        mut position: u64,
        previous: &str,
    ) -> Result<(), ProtocolError> {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(db_error(e)),
        };
        let metadata = file.metadata().map_err(db_error)?;
        if !metadata.is_file() {
            return Ok(());
        }
        // A small prefix detects replacement independently of appended file size.
        let mut prefix = vec![0; metadata.len().min(256) as usize];
        file.read_exact(&mut prefix).map_err(db_error)?;
        let signature = prefix
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut discard = previous.starts_with("skip:");
        let old = previous.strip_prefix("skip:").unwrap_or(previous);
        if position > metadata.len() || (!old.is_empty() && !signature.starts_with(old)) {
            position = 0;
            discard = false;
        }
        file.seek(SeekFrom::Start(position)).map_err(db_error)?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(db_error)?;
        let mut consumed = 0;
        for (index, byte) in bytes.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            if !discard {
                if let Ok(raw) = serde_json::from_slice::<Value>(&bytes[consumed..index]) {
                    self.claude_message(path, &raw)?;
                }
            }
            discard = false;
            consumed = index + 1;
        }
        if consumed == 0 && bytes.len() == 4 * 1024 * 1024 {
            consumed = bytes.len();
            discard = true;
            eprintln!("usage.transcript oversized record skipped");
        }
        let signature = if discard {
            format!("skip:{signature}")
        } else {
            signature
        };
        self.connection()?
            .execute(
                "UPDATE usage_sources SET position=?2,fingerprint=?3 WHERE path=?1",
                params![path, position + consumed as u64, signature],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn claude_message(&self, path: &str, raw: &Value) -> Result<(), ProtocolError> {
        let message = &raw["message"];
        let usage = &message["usage"];
        let Some(id) = message["id"].as_str() else {
            return Ok(());
        };
        if message["role"] != "assistant" || !usage.is_object() {
            return Ok(());
        }
        let Some(at) = raw["timestamp"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp())
        else {
            return Ok(());
        };
        let read = count(&usage["cache_read_input_tokens"]);
        let write = count(&usage["cache_creation_input_tokens"]);
        let input = sum3(count(&usage["input_tokens"]), read, write);
        let output = count(&usage["output_tokens"]);
        let total = input.zip(output).and_then(|(a, b)| a.checked_add(b));
        self.record(
            "observation",
            HOOK_DATASET,
            &format!("{}:{id}", raw["sessionId"].as_str().unwrap_or(path)),
            at.div_euclid(1800) * 1800,
            1800,
            message["model"].as_str(),
            [total, input, output, read, write],
        )
    }
    fn opencode_message(&self, raw: &Value) -> Result<(), ProtocolError> {
        if raw["type"] != "message.updated" {
            return Ok(());
        }
        let info = &raw["properties"]["info"];
        if info["role"] != "assistant" {
            return Ok(());
        }
        let (Some(id), Some(session), Some(at)) = (
            info["id"].as_str(),
            info["sessionID"].as_str(),
            info["time"]["completed"].as_i64(),
        ) else {
            return Ok(());
        };
        let tokens = &info["tokens"];
        let read = count(&tokens["cache"]["read"]);
        let write = count(&tokens["cache"]["write"]);
        let input = sum3(count(&tokens["input"]), read, write);
        // OpenCode stores reasoning separately from visible output; our output includes both.
        let output = count(&tokens["output"])
            .zip(count(&tokens["reasoning"]))
            .and_then(|(a, b)| a.checked_add(b));
        let model = info["modelID"].as_str().map(|model| {
            format!(
                "{}/{model}",
                info["providerID"].as_str().unwrap_or("unknown")
            )
        });
        self.record(
            "observation",
            HOOK_DATASET,
            &format!("{session}:{id}"),
            (at / 1000).div_euclid(1800) * 1800,
            1800,
            model.as_deref(),
            [
                input.zip(output).and_then(|(a, b)| a.checked_add(b)),
                input,
                output,
                read,
                write,
            ],
        )
    }

    pub fn import_codex_daily(&self, instance: &str, payload: &Value) -> Result<(), ProtocolError> {
        if payload["summary"].is_object() {
            let mut summary = json!({});
            for key in [
                "lifetimeTokens",
                "peakDailyTokens",
                "longestRunningTurnSec",
                "currentStreakDays",
                "longestStreakDays",
            ] {
                summary[key] = json!(count(&payload["summary"][key]));
            }
            self.connection()?.execute("INSERT INTO usage_native_summary VALUES(?1,?2) ON CONFLICT(instance) DO UPDATE SET payload=excluded.payload",params![instance,summary.to_string()]).map_err(db_error)?;
        }
        let Some(buckets) = payload["dailyUsageBuckets"].as_array() else {
            return if payload["summary"].is_object() {
                Ok(())
            } else {
                Err(error("usage_unavailable", "Codex returned no usage data"))
            };
        };
        for bucket in buckets {
            let date = chrono::NaiveDate::parse_from_str(
                bucket["startDate"].as_str().unwrap_or(""),
                "%Y-%m-%d",
            )
            .map_err(db_error)?;
            let Some(tokens) = count(&bucket["tokens"]) else {
                return Err(error("invalid_usage_record", "Invalid Codex daily tokens"));
            };
            self.record(
                instance,
                CODEX_DATASET,
                &date.to_string(),
                date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
                86400,
                None,
                [Some(tokens), None, None, None, None],
            )?;
        }
        Ok(())
    }
}
fn count(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|n| {
                n.is_finite() && *n >= 0.0 && *n <= 9_007_199_254_740_991.0 && n.fract() == 0.0
            })
            .map(|n| n as u64)
    })
}
fn sum3(a: Option<u64>, b: Option<u64>, c: Option<u64>) -> Option<u64> {
    a.zip(b)
        .zip(c)
        .and_then(|((a, b), c)| a.checked_add(b)?.checked_add(c))
}
