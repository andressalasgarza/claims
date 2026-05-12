mod core;
mod derived;
mod drift;
mod integrity;
mod validation;

pub use drift::{
    cascade_suspect, detect_dataset_drift, detect_drift, detect_rename,
    evidence_user_hash, hash_ref_if_local, latest_runnable_evidence, run_cmd,
};
pub use validation::{target_is_local, validate_evidence};

use rusqlite::Connection;
use std::path::PathBuf;

pub const STORE_DIR: &str = ".claims";
pub const INDEX_FILE: &str = "index.db";
pub(crate) const INTEGRITY_STRICT_MARKER: &str = ".integrity.strict";
pub(crate) const DEFAULT_INTEGRITY_KEY_DIR: &str = ".clms";
pub(crate) const DEFAULT_INTEGRITY_KEY_FILE: &str = "integrity.key";

pub struct Store {
    pub root: PathBuf,
    pub conn: Connection,
}

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS claims (
    seq         INTEGER PRIMARY KEY,
    ulid        TEXT NOT NULL UNIQUE,
    state       TEXT NOT NULL,
    confidence  TEXT,
    text        TEXT NOT NULL,
    tags        TEXT,
    agent       TEXT,
    session     TEXT,
    git_sha     TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_claims_state ON claims(state);
CREATE INDEX IF NOT EXISTS idx_claims_tags  ON claims(tags);

CREATE TABLE IF NOT EXISTS edges (
    from_seq INTEGER NOT NULL,
    to_seq   INTEGER NOT NULL,
    type     TEXT NOT NULL,
    PRIMARY KEY (from_seq, to_seq, type)
);
CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_seq);
"#;
