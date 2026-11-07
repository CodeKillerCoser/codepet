-- Source buckets use epoch seconds; updated/received use epoch milliseconds.
CREATE TABLE IF NOT EXISTS usage_records (
    instance TEXT NOT NULL,
    dataset TEXT NOT NULL,
    record_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    duration INTEGER NOT NULL,
    model TEXT,
    total_tokens INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    updated INTEGER NOT NULL,
    PRIMARY KEY(instance,dataset,record_id)
);

CREATE INDEX IF NOT EXISTS usage_time ON usage_records(instance,dataset,at);

CREATE TABLE IF NOT EXISTS usage_buckets (
    instance TEXT NOT NULL,
    dataset TEXT NOT NULL,
    at INTEGER NOT NULL,
    duration INTEGER NOT NULL,
    model TEXT NOT NULL,
    total_tokens INTEGER NOT NULL CHECK(total_tokens>=0),
    input_tokens INTEGER NOT NULL CHECK(input_tokens>=0),
    output_tokens INTEGER NOT NULL CHECK(output_tokens>=0),
    cache_read_tokens INTEGER NOT NULL CHECK(cache_read_tokens>=0),
    cache_write_tokens INTEGER NOT NULL CHECK(cache_write_tokens>=0),
    total_tokens_known_count INTEGER NOT NULL CHECK(total_tokens_known_count>=0),
    input_tokens_known_count INTEGER NOT NULL CHECK(input_tokens_known_count>=0),
    output_tokens_known_count INTEGER NOT NULL CHECK(output_tokens_known_count>=0),
    cache_read_tokens_known_count INTEGER NOT NULL CHECK(cache_read_tokens_known_count>=0),
    cache_write_tokens_known_count INTEGER NOT NULL CHECK(cache_write_tokens_known_count>=0),
    records INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    PRIMARY KEY(instance,dataset,at,model)
);

CREATE TABLE IF NOT EXISTS usage_native_summary (
    instance TEXT PRIMARY KEY NOT NULL,
    lifetime_tokens INTEGER,
    peak_daily_tokens INTEGER,
    longest_running_turn_seconds INTEGER,
    current_streak_days INTEGER,
    longest_streak_days INTEGER
);

CREATE TABLE IF NOT EXISTS usage_sources (
    path TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    prefix_fingerprint TEXT NOT NULL,
    skip_partial_line INTEGER NOT NULL DEFAULT 0 CHECK(skip_partial_line IN(0,1)),
    checked INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS usage_inbox (
    id TEXT PRIMARY KEY NOT NULL,
    source_record_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    model TEXT,
    total_tokens INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    received INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    dataset TEXT NOT NULL,
    generated_at TEXT NOT NULL,
    updated_at TEXT,
    scope TEXT NOT NULL,
    available_from TEXT,
    complete_through TEXT,
    coverage_status TEXT NOT NULL,
    summary_flags INTEGER NOT NULL,
    expires INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_snapshot_queries (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES usage_snapshots(id) ON DELETE CASCADE,
    instance TEXT NOT NULL,
    dataset TEXT NOT NULL,
    time_kind TEXT NOT NULL,
    range_from TEXT,
    range_to TEXT,
    time_zone TEXT NOT NULL,
    time_bucket TEXT NOT NULL,
    grouped INTEGER NOT NULL,
    include_unknown INTEGER,
    page_limit INTEGER NOT NULL,
    has_models INTEGER NOT NULL,
    has_summaries INTEGER NOT NULL,
    has_orders INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_snapshot_query_options (
    snapshot_id TEXT NOT NULL REFERENCES usage_snapshots(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN('model','metric','summary','order')),
    ordinal INTEGER NOT NULL,
    value TEXT NOT NULL,
    direction TEXT,
    PRIMARY KEY(snapshot_id,kind,ordinal)
);

CREATE TABLE IF NOT EXISTS usage_snapshot_entries (
    snapshot_id TEXT NOT NULL REFERENCES usage_snapshots(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN('row','total','group_total','peak','group_peak')),
    ordinal INTEGER NOT NULL,
    model_present INTEGER NOT NULL,
    model_id TEXT,
    bucket_from TEXT,
    bucket_to TEXT,
    provisional INTEGER,
    completeness TEXT,
    date TEXT,
    peak_total INTEGER,
    total_tokens INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    total_completeness TEXT,
    input_completeness TEXT,
    output_completeness TEXT,
    cache_read_completeness TEXT,
    cache_write_completeness TEXT,
    PRIMARY KEY(snapshot_id,kind,ordinal)
);

CREATE TABLE IF NOT EXISTS usage_snapshot_accounts (
    snapshot_id TEXT PRIMARY KEY NOT NULL REFERENCES usage_snapshots(id) ON DELETE CASCADE,
    lifetime_tokens INTEGER,
    peak_daily_tokens INTEGER,
    longest_running_turn_seconds INTEGER,
    current_streak_days INTEGER,
    longest_streak_days INTEGER
);
