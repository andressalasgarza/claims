use super::{harvest::harvest_all, Candidate, SuggestArgs, MAX_CEILING, SCHEMA_VERSION};
use crate::output::OutputFormat;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

pub(super) fn cmd_suggest(args: SuggestArgs, fmt: OutputFormat) -> Result<()> {
    let max = clamp_max(args.max)?;
    let sources_enabled = filter_sources(&args.source)?;
    let root = std::env::current_dir().context("get cwd")?;

    let (mut all, proposals_count) = harvest_all(&root)?;
    let actual = all.len();
    let truncated = actual.saturating_sub(max);
    all.truncate(max);

    let payload = build_suggest_payload(
        &all, max, actual, truncated, &sources_enabled, proposals_count,
    );
    emit_suggest_output(&payload, fmt, args.output, actual, truncated)
}

fn build_suggest_payload(
    candidates: &[Candidate],
    max: usize,
    actual: usize,
    truncated: usize,
    sources_enabled: &[String],
    proposals_count: usize,
) -> serde_json::Value {
    serde_json::json!({
        "version": SCHEMA_VERSION,
        "generated_at": Utc::now().to_rfc3339(),
        "harvester": {
            "max": max,
            "actual": candidates.len(),
            "extracted_total": actual,
            "truncated": truncated,
            "sources_enabled": sources_enabled,
            "from_source": actual - proposals_count,
            "from_proposals": proposals_count,
        },
        "candidates": candidates,
    })
}

fn emit_suggest_output(
    payload: &serde_json::Value,
    fmt: OutputFormat,
    output: Option<PathBuf>,
    actual: usize,
    truncated: usize,
) -> Result<()> {
    let serialized = if matches!(fmt, OutputFormat::Ai) {
        serde_json::to_string(payload)?
    } else {
        serde_json::to_string_pretty(payload)?
    };
    let Some(path) = output else {
        println!("{}", serialized);
        return Ok(());
    };
    fs::write(&path, &serialized)
        .with_context(|| format!("write {}", path.display()))?;
    if !matches!(fmt, OutputFormat::Ai) {
        eprintln!(
            "wrote {} candidates to {} (extracted {}, truncated {})",
            payload["harvester"]["actual"], path.display(), actual, truncated
        );
    }
    Ok(())
}

fn clamp_max(n: usize) -> Result<usize> {
    if n == 0 {
        bail!("--max must be > 0");
    }
    if n > MAX_CEILING {
        bail!(
            "--max {} exceeds ceiling {}. archaeology is bounded by design; if you need more candidates, tighten signal rules instead",
            n, MAX_CEILING
        );
    }
    Ok(n)
}

fn filter_sources(requested: &[String]) -> Result<Vec<String>> {
    let known = ["clms-claim-annotation"];
    if requested.is_empty() {
        return Ok(known.iter().map(|s| s.to_string()).collect());
    }
    let mut out = Vec::new();
    for r in requested {
        if !known.contains(&r.as_str()) {
            bail!(
                "unknown source '{}'. v2.0 ships only: {:?}. other signals deferred to v2.1",
                r, known
            );
        }
        if !out.contains(r) {
            out.push(r.clone());
        }
    }
    Ok(out)
}
