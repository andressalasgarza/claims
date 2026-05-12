//! archaeology v2 — candidate harvester + commit/purge
//!
//! see docs/archaeology.md for the full design. this module owns phase 1
//! (harvest) and phase 3 (commit), but NOT phase 2 (debate). debate happens
//! in pi-subagents (or any orchestrator that respects candidates.json /
//! survivors.json) — clms emits and ingests, never orchestrates.
//!
//! layout:
//!   archaeology.rs             -- dispatch + shared args/types/constants
//!   archaeology/suggest.rs     -- suggest command / output payload
//!   archaeology/harvest.rs     -- source annotation walking/scanning
//!   archaeology/proposals.rs   -- .archaeology/proposals.json surface
//!   archaeology/commit.rs      -- survivors.json -> pending claims
//!   archaeology/purge.rs       -- remove archaeology backfill sessions

mod commit;
mod harvest;
mod proposals;
mod purge;
mod suggest;

use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

pub(crate) const AGENT: &str = "archaeology";
pub(crate) const SCHEMA_VERSION: &str = "archaeology/v2";
pub(crate) const MAX_CEILING: usize = 50;

pub(crate) const CLAIM_MARKERS: &[&str] = &["// clms-claim:", "# clms-claim:"];
pub(crate) const EVIDENCE_MARKERS: &[&str] = &["// clms-evidence:", "# clms-evidence:"];

pub(crate) const SOURCE_EXTS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "go"];
pub(crate) const IGNORE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".claims",
    ".archaeology",
    ".pi",
    "venv",
    ".venv",
    "__pycache__",
    "dist",
    "build",
];

pub enum Sub {
    Suggest(SuggestArgs),
    Commit(CommitArgs),
    Purge(PurgeArgs),
}

pub struct SuggestArgs {
    pub max: usize,
    pub source: Vec<String>,
    pub output: Option<PathBuf>,
}

pub struct CommitArgs {
    pub from_plan: PathBuf,
    pub keep: usize,
}

pub struct PurgeArgs {
    pub session: String,
    pub agent: Option<String>,
}

pub fn dispatch(sub: Sub, store: &mut Store, fmt: OutputFormat) -> Result<()> {
    match sub {
        Sub::Suggest(a) => suggest::cmd_suggest(a, fmt),
        Sub::Commit(a) => commit::cmd_commit(store, a, fmt),
        Sub::Purge(a) => purge::cmd_purge(store, a, fmt),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) text: String,
    pub(crate) stake_signal: StakeSignal,
    pub(crate) suggested_evidence: Vec<SuggestedEvidence>,
    pub(crate) source_meta: SourceMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) created_at: Option<DateTime<Utc>>,
    pub(crate) debate: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct StakeSignal {
    pub(crate) r#where: String,
    pub(crate) snippet: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SuggestedEvidence {
    pub(crate) method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) r#ref: Option<String>,
    pub(crate) note: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SourceMeta {
    pub(crate) file: String,
    pub(crate) line: u32,
}
