CREATE TABLE IF NOT EXISTS conversation_inputs (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    owner TEXT NOT NULL,
    request_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    capability_revision TEXT NOT NULL,
    body TEXT NOT NULL,
    access_mode TEXT,
    reasoning_effort TEXT,
    model_kind TEXT,
    model_provider TEXT,
    model_id TEXT,
    state TEXT NOT NULL CHECK(state IN ('queued','dispatching','steering','accepted','blocked','outcomeUnknown','withdrawn')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    target_turn_id TEXT,
    error_code TEXT,
    error_message TEXT,
    withdraw_kind TEXT,
    withdraw_caller TEXT,
    edit_acknowledged INTEGER NOT NULL DEFAULT 0 CHECK(edit_acknowledged IN (0,1)),
    UNIQUE(conversation_id,owner,request_id),
    UNIQUE(conversation_id,sequence)
);
CREATE INDEX IF NOT EXISTS input_queue_pending ON conversation_inputs(conversation_id,state,sequence);
CREATE TABLE IF NOT EXISTS input_queue_operations (
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    caller TEXT NOT NULL,
    request_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    item_id TEXT NOT NULL REFERENCES conversation_inputs(id),
    target_turn_id TEXT,
    capability_revision TEXT,
    expected_revision INTEGER NOT NULL,
    PRIMARY KEY(conversation_id,caller,request_id)
);
CREATE TABLE IF NOT EXISTS input_queue_runtime (
    conversation_id INTEGER PRIMARY KEY REFERENCES conversations(id),
    active_turn_id TEXT
);
CREATE TABLE IF NOT EXISTS input_queue_terminal_turns (
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    turn_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('completed','failed','interrupted')),
    PRIMARY KEY(conversation_id,turn_id)
);
