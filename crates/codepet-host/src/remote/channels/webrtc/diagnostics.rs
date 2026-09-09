use ring::digest::{digest, SHA256};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use webrtc::peer_connection::RTCPeerConnection;

static SINK: OnceLock<mpsc::SyncSender<String>> = OnceLock::new();

/// A bounded asynchronous bridge to the embedding application's rotating log.
/// Call once during app startup. Diagnostics must never block RTC processing.
pub fn set_rtc_diagnostic_sink(sink: fn(&str)) {
    let (tx, rx) = mpsc::sync_channel::<String>(256);
    if SINK.set(tx).is_ok() {
        std::thread::spawn(move || {
            for line in rx {
                sink(&line);
            }
        });
    }
}

pub(super) fn id(value: &str) -> String {
    digest(&SHA256, value.as_bytes()).as_ref()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) fn address(value: &str, salt: &str) -> Value {
    let scope = match value.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let b = ip.octets();
            if ip.is_loopback() {
                "loopback"
            } else if ip.is_link_local() {
                "linkLocal"
            } else if ip.is_private() {
                "private"
            } else if b[0] == 100 && (64..=127).contains(&b[1]) {
                "cgnat"
            } else if b[0] == 198 && (18..=19).contains(&b[1]) {
                "benchmarkTun"
            } else {
                "public"
            }
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            if ip.is_loopback() {
                "loopback"
            } else if ip.is_unicast_link_local() {
                "linkLocal"
            } else if ip.is_unique_local() {
                "private"
            } else {
                "public"
            }
        }
        Err(_) => "hostname",
    };
    json!({"scope":scope,"addressId":id(&format!("{salt}:{value}"))})
}

pub(super) fn candidate(line: &str, salt: &str) -> Value {
    let f: Vec<_> = line
        .trim()
        .trim_start_matches("a=")
        .split_whitespace()
        .collect();
    if f.len() < 8 || !f[0].starts_with("candidate:") {
        return json!({"valid":false});
    }
    json!({"component": f[1].parse::<u32>().ok(),
        "protocol": if f[2].eq_ignore_ascii_case("udp") {"udp"} else if f[2].eq_ignore_ascii_case("tcp") {"tcp"} else {"unknown"},
        "priority":f[3].parse::<u32>().ok(), "address":address(f[4],salt), "port":f[5].parse::<u16>().ok(),
        "candidateType": if ["host","srflx","prflx","relay"].contains(&f[7]) {f[7]} else {"unknown"}})
}

pub(super) struct Diagnostic {
    pub offer_id: String,
    started: Instant,
}
impl Diagnostic {
    pub fn new(sdp: &str) -> Arc<Self> {
        Arc::new(Self {
            offer_id: id(sdp),
            started: Instant::now(),
        })
    }
    pub fn emit(&self, event: &str, fields: Value) {
        let line = json!({"schema":"codepet.rtc.v1","service":"host","offerId":self.offer_id,
            "timestampUnixMs":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "elapsedMs":self.started.elapsed().as_millis(),"event":event,"fields":fields}).to_string();
        if let Some(sink) = SINK.get() {
            let _ = sink.try_send(line);
        } else {
            eprintln!("{line}");
        }
    }
    pub fn description(&self, side: &str, sdp: &str) {
        let lines: Vec<_> = sdp
            .lines()
            .filter(|l| l.starts_with("a=candidate:"))
            .collect();
        self.emit("ice.description", json!({"side":side,"bytes":sdp.len(),"candidateCount":lines.len(),
            "truncated":lines.len()>128,"candidates":lines.iter().take(128).map(|l|candidate(l,&self.offer_id)).collect::<Vec<_>>()}));
    }
    pub async fn stats(&self, peer: &RTCPeerConnection, reason: &str) {
        let Ok(stats) =
            tokio::time::timeout(std::time::Duration::from_secs(1), peer.get_stats()).await
        else {
            self.emit("ice.statsUnavailable", json!({"reason":reason}));
            return;
        };
        let mut reports = Vec::new();
        for report in stats.reports.values() {
            let Ok(v) = serde_json::to_value(report) else {
                continue;
            };
            if ![
                "candidate-pair",
                "local-candidate",
                "remote-candidate",
                "transport",
                "data-channel",
            ]
            .contains(&v["type"].as_str().unwrap_or(""))
            {
                continue;
            }
            reports.push(sanitize_stat(&v, &self.offer_id));
        }
        self.emit(
            "ice.stats",
            json!({"reason":reason,"count":reports.len(),"truncated":reports.len()>128,
            "reports":reports.iter().take(128).collect::<Vec<_>>()}),
        );
    }
}

fn sanitize_stat(v: &Value, salt: &str) -> Value {
    let mut result = serde_json::Map::new();
    for key in [
        "id",
        "type",
        "state",
        "nominated",
        "localCandidateId",
        "remoteCandidateId",
        "selectedCandidatePairId",
        "candidateType",
        "networkType",
        "relayProtocol",
        "port",
        "priority",
        "bytesSent",
        "bytesReceived",
        "packetsSent",
        "packetsReceived",
        "requestsSent",
        "requestsReceived",
        "responsesSent",
        "responsesReceived",
        "consentRequestsSent",
        "retransmissionsSent",
        "currentRoundTripTime",
        "totalRoundTripTime",
        "availableOutgoingBitrate",
        "messagesSent",
        "messagesReceived",
    ] {
        if let Some(value) = v.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    // webrtc-ice 0.14 only fills IDs/state/nominated; the other pair fields are defaults.
    // A placeholder zero must not be presented as proof that no check was sent.
    if v["type"] == "candidate-pair" {
        result.retain(|key, _| {
            [
                "id",
                "type",
                "state",
                "nominated",
                "localCandidateId",
                "remoteCandidateId",
            ]
            .contains(&key.as_str())
        });
        result.insert("countersAvailable".into(), Value::Bool(false));
    }
    if let Some(ip) = v["ip"].as_str() {
        result.insert("address".into(), address(ip, salt));
    }
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_exclude_addresses_credentials_and_unknown_fields() {
        let value = candidate(
            "candidate:a 1 udp 123 198.18.0.1 333 typ host ufrag secret",
            "offer",
        )
        .to_string();
        assert!(value.contains("benchmarkTun"));
        assert!(!value.contains("198.18.0.1") && !value.contains("secret"));
        let stats = sanitize_stat(
            &json!({"ip":"203.0.113.1","url":"turn:user:password@host","username":"secret","requestsSent":2}),
            "offer",
        );
        assert_eq!(stats["requestsSent"], 2);
        assert!(stats.get("url").is_none() && stats.get("username").is_none());
    }
}
