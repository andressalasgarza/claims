mod build;
mod candidates;
mod plan;
mod report;

use super::CommitArgs;
use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::{anyhow, bail, Context, Result};
use candidates::{process_candidates, scan_existing_candidate_ids};
use chrono::Utc;
use plan::load_and_validate_plan;
use report::emit_commit_report;
use std::fs;
use std::path::PathBuf;

pub(super) fn cmd_commit(store: &mut Store, args: CommitArgs, fmt: OutputFormat) -> Result<()> {
    if args.keep == 0 {
        bail!("--keep must be > 0");
    }
    let plan = load_and_validate_plan(&args.from_plan, args.keep)?;
    let session = format!("backfill-{}", Utc::now().to_rfc3339());
    let archaeology_dir = PathBuf::from(".archaeology").join(&session);
    fs::create_dir_all(&archaeology_dir)
        .with_context(|| format!("create {}", archaeology_dir.display()))?;

    let existing_ids = scan_existing_candidate_ids(store)?;
    let candidates = plan["candidates"]
        .as_array()
        .ok_or_else(|| anyhow!("survivors.json: 'candidates' must be an array"))?;

    let outcome = process_candidates(
        store, candidates, &session, &archaeology_dir, &existing_ids,
    )?;

    emit_commit_report(fmt, &session, &archaeology_dir, &outcome, args.keep)
}

pub(super) struct CommitOutcome {
    written: usize,
    skipped: usize,
    summary: Vec<serde_json::Value>,
}

