use crate::agents::AgentId;
use crate::events::PetEvent;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSession {
    pub provider: AgentId,
    pub session_id: String,
    pub source_path: String,
    pub fingerprint: SourceFingerprint,
    pub day: String,
    #[serde(default)]
    pub buckets: Vec<TokenUsageBucket>,
    pub models: Vec<String>,
    pub usage: TokenUsage,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageBucket {
    pub bucket_start: String,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFingerprint {
    pub len: u64,
    pub modified_ms: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageStore {
    #[serde(default)]
    pub sessions: BTreeMap<String, TokenUsageSession>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageSummary {
    pub provider: AgentId,
    pub sessions: usize,
    pub total: TokenUsage,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsageSummary {
    pub day: String,
    pub sessions: usize,
    pub total: TokenUsage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketUsageSummary {
    pub provider: AgentId,
    pub bucket_start: String,
    pub sessions: usize,
    pub total: TokenUsage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSessionSummary {
    pub provider: AgentId,
    pub session_id: String,
    pub day: String,
    pub models: Vec<String>,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSummary {
    pub total: TokenUsage,
    pub by_provider: Vec<ProviderUsageSummary>,
    pub by_day: Vec<DailyUsageSummary>,
    pub by_bucket: Vec<BucketUsageSummary>,
    pub sessions: Vec<TokenUsageSessionSummary>,
}

impl TokenUsageStore {
    pub fn summary(&self) -> TokenUsageSummary {
        let mut total = TokenUsage::default();
        let mut by_provider = BTreeMap::<String, ProviderUsageSummary>::new();
        let mut by_day = BTreeMap::<String, DailyUsageSummary>::new();
        let mut by_bucket = BTreeMap::<String, BucketUsageSummary>::new();
        let mut sessions = Vec::new();

        for session in self.sessions.values() {
            add_usage(&mut total, &session.usage);
            let provider_key = session.provider.as_str().to_string();
            let provider_entry = by_provider.entry(provider_key).or_insert_with(|| ProviderUsageSummary {
                provider: session.provider,
                sessions: 0,
                total: TokenUsage::default(),
            });
            provider_entry.sessions += 1;
            add_usage(&mut provider_entry.total, &session.usage);

            let day_entry = by_day.entry(session.day.clone()).or_insert_with(|| DailyUsageSummary {
                day: session.day.clone(),
                sessions: 0,
                total: TokenUsage::default(),
            });
            day_entry.sessions += 1;
            add_usage(&mut day_entry.total, &session.usage);

            let buckets = if session.buckets.is_empty() {
                vec![TokenUsageBucket {
                    bucket_start: bucket_start_from_day(&session.day),
                    usage: session.usage.clone(),
                }]
            } else {
                session.buckets.clone()
            };
            for bucket in buckets {
                let bucket_key = format!("{}:{}", bucket.bucket_start, session.provider.as_str());
                let bucket_entry = by_bucket.entry(bucket_key).or_insert_with(|| BucketUsageSummary {
                    provider: session.provider,
                    bucket_start: bucket.bucket_start.clone(),
                    sessions: 0,
                    total: TokenUsage::default(),
                });
                bucket_entry.sessions += 1;
                add_usage(&mut bucket_entry.total, &bucket.usage);
            }

            sessions.push(TokenUsageSessionSummary {
                provider: session.provider,
                session_id: session.session_id.clone(),
                day: session.day.clone(),
                models: session.models.clone(),
                usage: session.usage.clone(),
            });
        }

        sessions.sort_by(|left, right| right.day.cmp(&left.day).then_with(|| left.provider.as_str().cmp(right.provider.as_str())));

        TokenUsageSummary {
            total,
            by_provider: by_provider.into_values().collect(),
            by_day: by_day.into_values().rev().collect(),
            by_bucket: by_bucket.into_values().collect(),
            sessions,
        }
    }
}

pub fn refresh_default_usage_summary() -> io::Result<TokenUsageSummary> {
    let path = default_store_path();
    let mut store = load_usage_store_from(&path)?;
    refresh_known_sources(&mut store)?;
    save_usage_store_to(&path, &store)?;
    Ok(store.summary())
}

pub fn load_default_usage_summary() -> io::Result<TokenUsageSummary> {
    Ok(load_usage_store_from(&default_store_path())?.summary())
}

pub fn refresh_usage_for_event(event: &PetEvent) -> io::Result<Option<TokenUsageSummary>> {
    let Some(path) = transcript_path_from_event(event) else {
        return Ok(None);
    };
    let store_path = default_store_path();
    let mut store = load_usage_store_from(&store_path)?;
    if refresh_transcript_usage(&mut store, event.provider, &path, event.session_id.as_deref())? {
        save_usage_store_to(&store_path, &store)?;
        return Ok(Some(store.summary()));
    }
    Ok(None)
}

pub fn load_usage_store_from(path: &Path) -> io::Result<TokenUsageStore> {
    if !path.exists() {
        return Ok(TokenUsageStore::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text).unwrap_or_default())
}

pub fn save_usage_store_to(path: &Path, store: &TokenUsageStore) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(store)?)
}

pub fn refresh_known_sources(store: &mut TokenUsageStore) -> io::Result<usize> {
    let mut changed = 0;
    for (provider, path) in default_audit_paths() {
        for path in transcript_paths_from_audit(&path)? {
            if refresh_transcript_usage(store, provider, &path, None)? {
                changed += 1;
            }
        }
    }
    for (provider, root) in default_recursive_source_roots() {
        for path in jsonl_files_under(&root)? {
            if refresh_transcript_usage(store, provider, &path, None)? {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

pub fn refresh_transcript_usage(
    store: &mut TokenUsageStore,
    provider: AgentId,
    path: &Path,
    session_id_hint: Option<&str>,
) -> io::Result<bool> {
    let fingerprint = source_fingerprint(path)?;
    let fallback_session_id = session_id_hint.map(ToString::to_string).or_else(|| session_id_from_path(path));
    if let Some(session_id) = fallback_session_id.as_deref() {
        let key = session_key(provider, session_id);
        if store
            .sessions
            .get(&key)
            .is_some_and(|session| session.source_path == path.to_string_lossy() && session.fingerprint == fingerprint && !session.buckets.is_empty())
        {
            return Ok(false);
        }
    }

    let Some(parsed) = parse_transcript_usage(provider, path, session_id_hint, &fingerprint)? else {
        return Ok(false);
    };
    let key = session_key(provider, &parsed.session_id);
    let changed = store
        .sessions
        .get(&key)
        .map(|current| current.usage != parsed.usage || current.fingerprint != parsed.fingerprint || current.buckets != parsed.buckets)
        .unwrap_or(true);
    store.sessions.insert(key, parsed);
    Ok(changed)
}

fn parse_transcript_usage(
    provider: AgentId,
    path: &Path,
    session_id_hint: Option<&str>,
    fingerprint: &SourceFingerprint,
) -> io::Result<Option<TokenUsageSession>> {
    match provider {
        AgentId::Codex => parse_codex_usage(path, session_id_hint, fingerprint),
        AgentId::Claude | AgentId::Qoder | AgentId::Cursor => parse_assistant_message_usage(provider, path, session_id_hint, fingerprint),
    }
}

fn parse_codex_usage(
    path: &Path,
    session_id_hint: Option<&str>,
    fingerprint: &SourceFingerprint,
) -> io::Result<Option<TokenUsageSession>> {
    let text = fs::read_to_string(path)?;
    let mut latest = None;
    let mut previous = None;
    let mut day = None;
    let mut buckets = BTreeMap::<String, TokenUsage>::new();
    let mut models = BTreeSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if day.is_none() {
            day = day_from_value(&value);
        }
        let timestamp = timestamp_from_value(&value);
        if let Some(model) = string_at(&value, &["payload", "model"]).or_else(|| string_at(&value, &["model"])) {
            models.insert(model);
        }
        if let Some(usage) = value
            .pointer("/payload/info/total_token_usage")
            .and_then(usage_from_codex_total)
        {
            let delta = previous.as_ref().map_or_else(|| usage.clone(), |previous| subtract_usage(&usage, previous));
            if has_any_usage(&delta) {
                let bucket_start = timestamp
                    .map(bucket_start_from_timestamp)
                    .unwrap_or_else(|| bucket_start_from_fingerprint(fingerprint));
                add_usage(buckets.entry(bucket_start).or_default(), &delta);
            }
            previous = Some(usage.clone());
            latest = Some(usage);
        }
    }

    let Some(usage) = latest else {
        return Ok(None);
    };
    Ok(Some(TokenUsageSession {
        provider: AgentId::Codex,
        session_id: session_id_hint.map(ToString::to_string).or_else(|| session_id_from_path(path)).unwrap_or_else(|| path.to_string_lossy().to_string()),
        source_path: path.to_string_lossy().to_string(),
        fingerprint: fingerprint.clone(),
        day: day.unwrap_or_else(|| day_from_fingerprint(fingerprint)),
        buckets: buckets
            .into_iter()
            .map(|(bucket_start, usage)| TokenUsageBucket { bucket_start, usage })
            .collect(),
        models: models.into_iter().collect(),
        usage,
        updated_at: Utc::now(),
    }))
}

fn parse_assistant_message_usage(
    provider: AgentId,
    path: &Path,
    session_id_hint: Option<&str>,
    fingerprint: &SourceFingerprint,
) -> io::Result<Option<TokenUsageSession>> {
    let text = fs::read_to_string(path)?;
    let mut usage = TokenUsage::default();
    let mut buckets = BTreeMap::<String, TokenUsage>::new();
    let mut seen = HashSet::new();
    let mut session_id = session_id_hint.map(ToString::to_string);
    let mut day = None;
    let mut models = BTreeSet::new();
    let mut counted = 0;

    for (index, line) in text.lines().filter(|line| !line.trim().is_empty()).enumerate() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if session_id.is_none() {
            session_id = first_string(&value, &["sessionId", "session_id"]);
        }
        if day.is_none() {
            day = day_from_value(&value);
        }
        let timestamp = timestamp_from_value(&value);
        if let Some(model) = string_at(&value, &["message", "model"]).or_else(|| string_at(&value, &["model"])) {
            models.insert(model);
        }

        let Some(message_usage) = value.pointer("/message/usage").and_then(usage_from_assistant_message) else {
            continue;
        };
        let usage_key = string_at(&value, &["message", "usage", "request_id"])
            .or_else(|| string_at(&value, &["message", "id"]))
            .or_else(|| string_at(&value, &["uuid"]))
            .unwrap_or_else(|| format!("line-{index}"));
        if !seen.insert(usage_key) {
            continue;
        }
        add_usage(&mut usage, &message_usage);
        let bucket_start = timestamp
            .map(bucket_start_from_timestamp)
            .unwrap_or_else(|| bucket_start_from_fingerprint(fingerprint));
        add_usage(buckets.entry(bucket_start).or_default(), &message_usage);
        counted += 1;
    }

    if counted == 0 {
        return Ok(None);
    }
    normalize_total_tokens(&mut usage);
    Ok(Some(TokenUsageSession {
        provider,
        session_id: session_id.or_else(|| session_id_from_path(path)).unwrap_or_else(|| path.to_string_lossy().to_string()),
        source_path: path.to_string_lossy().to_string(),
        fingerprint: fingerprint.clone(),
        day: day.unwrap_or_else(|| day_from_fingerprint(fingerprint)),
        buckets: buckets
            .into_iter()
            .map(|(bucket_start, usage)| TokenUsageBucket { bucket_start, usage })
            .collect(),
        models: models.into_iter().collect(),
        usage,
        updated_at: Utc::now(),
    }))
}

fn usage_from_codex_total(value: &Value) -> Option<TokenUsage> {
    let mut usage = TokenUsage {
        input_tokens: number(value, "input_tokens"),
        cached_input_tokens: number(value, "cached_input_tokens"),
        output_tokens: number(value, "output_tokens"),
        reasoning_output_tokens: number(value, "reasoning_output_tokens"),
        total_tokens: number(value, "total_tokens"),
        ..TokenUsage::default()
    };
    if usage.total_tokens == 0 {
        normalize_total_tokens(&mut usage);
    }
    has_any_usage(&usage).then_some(usage)
}

fn usage_from_assistant_message(value: &Value) -> Option<TokenUsage> {
    let mut usage = TokenUsage {
        input_tokens: number(value, "input_tokens"),
        output_tokens: number(value, "output_tokens"),
        cache_creation_input_tokens: number(value, "cache_creation_input_tokens"),
        cache_read_input_tokens: number(value, "cache_read_input_tokens"),
        ..TokenUsage::default()
    };
    normalize_total_tokens(&mut usage);
    has_any_usage(&usage).then_some(usage)
}

fn add_usage(total: &mut TokenUsage, usage: &TokenUsage) {
    total.input_tokens += usage.input_tokens;
    total.cached_input_tokens += usage.cached_input_tokens;
    total.output_tokens += usage.output_tokens;
    total.reasoning_output_tokens += usage.reasoning_output_tokens;
    total.cache_creation_input_tokens += usage.cache_creation_input_tokens;
    total.cache_read_input_tokens += usage.cache_read_input_tokens;
    total.total_tokens += usage.total_tokens;
}

fn subtract_usage(current: &TokenUsage, previous: &TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: current.input_tokens.saturating_sub(previous.input_tokens),
        cached_input_tokens: current.cached_input_tokens.saturating_sub(previous.cached_input_tokens),
        output_tokens: current.output_tokens.saturating_sub(previous.output_tokens),
        reasoning_output_tokens: current.reasoning_output_tokens.saturating_sub(previous.reasoning_output_tokens),
        cache_creation_input_tokens: current.cache_creation_input_tokens.saturating_sub(previous.cache_creation_input_tokens),
        cache_read_input_tokens: current.cache_read_input_tokens.saturating_sub(previous.cache_read_input_tokens),
        total_tokens: current.total_tokens.saturating_sub(previous.total_tokens),
    }
}

fn normalize_total_tokens(usage: &mut TokenUsage) {
    if usage.total_tokens == 0 {
        usage.total_tokens = usage.input_tokens + usage.output_tokens;
    }
}

fn has_any_usage(usage: &TokenUsage) -> bool {
    usage.input_tokens > 0
        || usage.cached_input_tokens > 0
        || usage.output_tokens > 0
        || usage.reasoning_output_tokens > 0
        || usage.cache_creation_input_tokens > 0
        || usage.cache_read_input_tokens > 0
        || usage.total_tokens > 0
}

fn source_fingerprint(path: &Path) -> io::Result<SourceFingerprint> {
    let metadata = fs::metadata(path)?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default();
    Ok(SourceFingerprint {
        len: metadata.len(),
        modified_ms,
    })
}

fn day_from_value(value: &Value) -> Option<String> {
    first_string(value, &["timestamp", "_timestamp", "created_at", "createdAt", "ts"]).and_then(|value| value.get(0..10).map(ToString::to_string))
}

fn day_from_fingerprint(fingerprint: &SourceFingerprint) -> String {
    DateTime::<Utc>::from_timestamp_millis(fingerprint.modified_ms)
        .map(|value| value.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string())
}

fn timestamp_from_value(value: &Value) -> Option<DateTime<Utc>> {
    first_string(value, &["timestamp", "_timestamp", "created_at", "createdAt", "ts"])
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn bucket_start_from_timestamp(timestamp: DateTime<Utc>) -> String {
    let bucket_seconds = timestamp.timestamp().div_euclid(30 * 60) * 30 * 60;
    DateTime::<Utc>::from_timestamp(bucket_seconds, 0)
        .unwrap_or(timestamp)
        .format("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string()
}

fn bucket_start_from_fingerprint(fingerprint: &SourceFingerprint) -> String {
    DateTime::<Utc>::from_timestamp_millis(fingerprint.modified_ms)
        .map(bucket_start_from_timestamp)
        .unwrap_or_else(|| bucket_start_from_timestamp(Utc::now()))
}

fn bucket_start_from_day(day: &str) -> String {
    format!("{day}T00:00:00+00:00")
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(ToString::to_string))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToString::to_string)
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if let Some(rest) = stem.strip_prefix("rollout-") {
        let parts = rest.split('-').collect::<Vec<_>>();
        if parts.len() >= 5 {
            return Some(parts[parts.len() - 5..].join("-"));
        }
    }
    Some(stem.to_string())
}

fn session_key(provider: AgentId, session_id: &str) -> String {
    format!("{}:{session_id}", provider.as_str())
}

fn default_audit_paths() -> Vec<(AgentId, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        (AgentId::Codex, home.join(".codex").join("audit").join("audit.jsonl")),
        (AgentId::Qoder, home.join(".qoder").join("audit").join("audit.jsonl")),
    ]
}

fn default_recursive_source_roots() -> Vec<(AgentId, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![(AgentId::Claude, home.join(".claude").join("projects"))]
}

fn transcript_paths_from_audit(path: &Path) -> io::Result<Vec<PathBuf>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(path) = value.get("transcript_path").and_then(Value::as_str).map(PathBuf::from) else {
            continue;
        };
        if path.is_absolute() && path.exists() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn transcript_path_from_event(event: &PetEvent) -> Option<PathBuf> {
    let value = event.raw.get("transcript_path").and_then(Value::as_str)?;
    let path = Path::new(value);
    path.is_absolute().then(|| path.to_path_buf())
}

fn jsonl_files_under(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files)?;
    Ok(files)
}

fn collect_jsonl_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        collect_jsonl_files(&entry.path(), files)?;
    }
    Ok(())
}

fn default_store_path() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("code-pet")
        .join("token-usage.json")
}
