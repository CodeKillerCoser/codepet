CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)>0),
    display_name TEXT NOT NULL,
    selected_executable_path TEXT,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision>=0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_instances (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)>0),
    provider_id TEXT NOT NULL REFERENCES providers(id),
    instance_kind TEXT NOT NULL,
    display_name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS host_identity (
    id INTEGER PRIMARY KEY CHECK(id=1),
    device_id TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS registry_identity (
    id INTEGER PRIMARY KEY CHECK(id=1), device_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS activity_scopes (
    id TEXT PRIMARY KEY NOT NULL, last_version INTEGER NOT NULL CHECK(last_version>=0)
);
INSERT OR IGNORE INTO activity_scopes VALUES('host',0);
CREATE TABLE IF NOT EXISTS conversations (
    id INTEGER PRIMARY KEY,
    provider_instance_id TEXT NOT NULL REFERENCES provider_instances(id),
    native_conversation_id TEXT NOT NULL CHECK(length(native_conversation_id)>0),
    title TEXT,
    activity_scope_id TEXT NOT NULL REFERENCES activity_scopes(id),
    activity_version INTEGER NOT NULL CHECK(activity_version>=0),
    summary_fingerprint TEXT,
    detail_fingerprint TEXT,
    last_activity_event_key TEXT,
    queue_paused INTEGER NOT NULL DEFAULT 0 CHECK(queue_paused IN(0,1)),
    queue_revision INTEGER NOT NULL DEFAULT 0 CHECK(queue_revision>=0),
    next_queue_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_queue_sequence>=1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(provider_instance_id,native_conversation_id)
);
CREATE INDEX IF NOT EXISTS conversation_activity ON conversations(activity_scope_id,activity_version);
CREATE TABLE IF NOT EXISTS reader_baselines (
    scope_id TEXT NOT NULL REFERENCES activity_scopes(id),
    reader_id TEXT NOT NULL,
    baseline_version INTEGER NOT NULL CHECK(baseline_version>=0),
    PRIMARY KEY(scope_id,reader_id)
);
CREATE TABLE IF NOT EXISTS conversation_reads (
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    reader_id TEXT NOT NULL,
    read_version INTEGER NOT NULL CHECK(read_version>=0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(conversation_id,reader_id)
);
CREATE TABLE IF NOT EXISTS host_tls_identity (
    id INTEGER PRIMARY KEY CHECK(id=1),
    certificate_der BLOB NOT NULL,
    private_key_der BLOB NOT NULL,
    certificate_sha256 TEXT NOT NULL UNIQUE,
    private_key_sha256 TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS remote_credentials (
    id TEXT PRIMARY KEY NOT NULL,
    client_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    operating_system TEXT NOT NULL,
    system_version TEXT NOT NULL,
    bearer_sha256 TEXT NOT NULL UNIQUE,
    tls_certificate_sha256 TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE TABLE IF NOT EXISTS signaling_identity (
    id INTEGER PRIMARY KEY CHECK(id=1), signing_seed BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS signaling_services (
    service_url TEXT PRIMARY KEY NOT NULL,
    host_token TEXT NOT NULL,
    auto_register INTEGER NOT NULL CHECK(auto_register IN(0,1))
);
CREATE TABLE IF NOT EXISTS signaling_peers (
    service_url TEXT NOT NULL REFERENCES signaling_services(service_url),
    credential_id TEXT NOT NULL,
    public_key BLOB NOT NULL,
    token TEXT NOT NULL,
    PRIMARY KEY(service_url,credential_id)
);
