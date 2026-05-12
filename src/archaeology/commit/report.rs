use super::super::AGENT;
use super::CommitOutcome;
use crate::output::OutputFormat;
use anyhow::Result;
use std::path::Path;

pub(super) fn emit_commit_report(
    fmt: OutputFormat,
    session: &str,
    archaeology_dir: &Path,
    outcome: &CommitOutcome,
    keep_cap: usize,
) -> Result<()> {
    let report = serde_json::json!({
        "agent": AGENT,
        "session": session,
        "transcript_dir": archaeology_dir.display().to_string(),
        "written": outcome.written,
        "skipped_idempotent": outcome.skipped,
        "keep_cap": keep_cap,
        "claims": outcome.summary,
    });
    if matches!(fmt, OutputFormat::Ai) {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    println!(
        "archaeology commit: session={} written={} skipped_idempotent={} (cap K={})",
        session, outcome.written, outcome.skipped, keep_cap
    );
    println!("transcripts: {}", archaeology_dir.display());
    Ok(())
}

