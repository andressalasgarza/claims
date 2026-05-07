//! archaeology v2 — candidate harvester + (later) commit/purge
//!
//! see docs/archaeology.md for the full design. this module owns phase 1
//! (harvest) and phase 3 (commit), but NOT phase 2 (debate). debate happens
//! in pi-subagents (or any orchestrator that respects candidates.json /
//! survivors.json) — clms emits and ingests, never orchestrates.
//!
//! v2.0 ships ONE signal: `clms-claim-annotation`. byte-walk for
//! `// clms-claim:` and `# clms-claim:` source comments. other signals
//! (test-name-invariant, type-marked, mb-verify-task, cm-structural) are
//! deferred to v2.1 once dogfood data shows whether they're needed.

use crate::output::OutputFormat;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const AGENT: &str = "archaeology";
pub const SCHEMA_VERSION: &str = "archaeology/v2";
pub const DEFAULT_MAX: usize = 10;
pub const MAX_CEILING: usize = 50;

const CLAIM_MARKERS: &[&str] = &["// clms-claim:", "# clms-claim:"];
const EVIDENCE_MARKERS: &[&str] = &["// clms-evidence:", "# clms-evidence:"];

const SOURCE_EXTS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "go", "md"];
const IGNORE_DIRS: &[&str] = &[
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

// ---------- subcommand dispatch ----------

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

pub fn dispatch(sub: Sub, fmt: OutputFormat) -> Result<()> {
    match sub {
        Sub::Suggest(a) => cmd_suggest(a, fmt),
        Sub::Commit(_) => bail!(
            "archaeology commit not yet implemented in this build (m-e4ce). \
             use `clms archaeology suggest` to harvest candidates."
        ),
        Sub::Purge(_) => bail!(
            "archaeology purge not yet implemented in this build (m-2313)."
        ),
    }
}

// ---------- candidate types ----------

#[derive(Debug, Clone, serde::Serialize)]
pub struct Candidate {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub stake_signal: StakeSignal,
    pub suggested_evidence: Vec<SuggestedEvidence>,
    pub source_meta: SourceMeta,
    pub debate: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StakeSignal {
    pub r#where: String,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SuggestedEvidence {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceMeta {
    pub file: String,
    pub line: u32,
}

// ---------- harvest ----------

fn cmd_suggest(args: SuggestArgs, fmt: OutputFormat) -> Result<()> {
    let max = clamp_max(args.max)?;
    let sources_enabled = filter_sources(&args.source)?;
    let root = std::env::current_dir().context("get cwd")?;

    let mut all = harvest_annotations(&root)?;
    all.sort_by(|a, b| {
        (&a.source_meta.file, a.source_meta.line)
            .cmp(&(&b.source_meta.file, b.source_meta.line))
    });
    let actual = all.len();
    let truncated = if all.len() > max { all.len() - max } else { 0 };
    all.truncate(max);

    let payload = serde_json::json!({
        "version": SCHEMA_VERSION,
        "generated_at": Utc::now().to_rfc3339(),
        "harvester": {
            "max": max,
            "actual": all.len(),
            "extracted_total": actual,
            "truncated": truncated,
            "sources_enabled": sources_enabled,
        },
        "candidates": all,
    });

    let serialized = if matches!(fmt, OutputFormat::Ai) {
        serde_json::to_string(&payload)?
    } else {
        serde_json::to_string_pretty(&payload)?
    };

    match args.output {
        Some(path) => {
            fs::write(&path, &serialized)
                .with_context(|| format!("write {}", path.display()))?;
            if !matches!(fmt, OutputFormat::Ai) {
                eprintln!(
                    "wrote {} candidates to {} (extracted {}, truncated {})",
                    payload["harvester"]["actual"], path.display(), actual, truncated
                );
            }
        }
        None => println!("{}", serialized),
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

fn harvest_annotations(root: &Path) -> Result<Vec<Candidate>> {
    let mut files = Vec::new();
    walk_dir(root, root, &mut files)?;
    files.sort();

    let mut candidates = Vec::new();
    for f in &files {
        let raw = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue, // binary, permission denied, etc.
        };
        scan_file(f, root, &raw, &mut candidates);
    }
    Ok(candidates)
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if IGNORE_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            walk_dir(root, &path, out)?;
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SOURCE_EXTS.contains(&ext) {
                    out.push(path);
                }
            }
        }
    }
    Ok(())
}

fn scan_file(path: &Path, root: &Path, content: &str, out: &mut Vec<Candidate>) {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let lines: Vec<&str> = content.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if let Some((marker, claim_text)) = match_marker(line, CLAIM_MARKERS) {
            let claim_text = claim_text.trim().to_string();
            if claim_text.is_empty() {
                continue;
            }
            let snippet = line.trim().to_string();
            let where_str = format!("{}:{}", rel, idx + 1);

            // optional adjacent evidence marker on next line
            let mut suggested_evidence = Vec::new();
            if let Some(next) = lines.get(idx + 1) {
                if let Some((_, ev_text)) = match_marker(next, EVIDENCE_MARKERS) {
                    suggested_evidence.push(parse_evidence_directive(ev_text));
                }
            }

            let id = candidate_id("clms-claim-annotation", &claim_text, &where_str);
            out.push(Candidate {
                id,
                kind: "clms-claim-annotation".into(),
                text: claim_text,
                stake_signal: StakeSignal {
                    r#where: where_str,
                    snippet,
                },
                suggested_evidence,
                source_meta: SourceMeta {
                    file: rel.clone(),
                    line: (idx as u32) + 1,
                },
                debate: None,
            });
            // marker variable preserves which prefix matched if useful later
            let _ = marker;
        }
    }
}

fn match_marker<'a>(line: &'a str, markers: &[&str]) -> Option<(&'static str, &'a str)> {
    let trimmed = line.trim_start();
    for m in markers {
        if let Some(rest) = trimmed.strip_prefix(m) {
            // Match is cheap; we re-resolve marker text from the constant slice.
            // Find which static marker matched.
            for static_m in CLAIM_MARKERS.iter().chain(EVIDENCE_MARKERS.iter()) {
                if static_m == m {
                    return Some((static_m, rest));
                }
            }
        }
    }
    None
}

fn parse_evidence_directive(raw: &str) -> SuggestedEvidence {
    // simple key=value parsing, no quoting. format:
    //   method=<name> [cmd=<...>] [ref=<...>]
    let mut method = "code-test".to_string();
    let mut cmd: Option<String> = None;
    let mut r#ref: Option<String> = None;
    let trimmed = raw.trim();
    for token in split_kv(trimmed) {
        if let Some((k, v)) = token.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "method" => method = v,
                "cmd" => cmd = Some(v),
                "ref" => r#ref = Some(v),
                _ => {}
            }
        }
    }
    SuggestedEvidence {
        method,
        cmd,
        r#ref,
        note: "advisory only; not run by archaeology. promotion via `clms verify`".into(),
    }
}

fn split_kv(s: &str) -> Vec<String> {
    // split on whitespace not inside quotes
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        if c == '"' {
            in_q = !in_q;
            cur.push(c);
        } else if c.is_whitespace() && !in_q {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn candidate_id(kind: &str, text: &str, where_str: &str) -> String {
    let payload = format!("{}|{}|{}", kind, text, where_str);
    let h = blake3::hash(payload.as_bytes()).to_hex();
    format!("c-{}", &h.to_string()[..4])
}

// ---------- helpers used elsewhere ----------

#[allow(dead_code)]
pub fn agent_excluded(agent: &Option<String>, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    let set: HashSet<&str> = exclude.iter().map(|s| s.as_str()).collect();
    agent.as_deref().map(|a| set.contains(a)).unwrap_or(false)
}

#[allow(dead_code)]
pub fn shadow_session() -> String {
    format!("backfill-{}", Utc::now().to_rfc3339())
}

#[allow(dead_code)]
pub fn _ensure_unused() -> Result<()> {
    Err(anyhow!("placeholder"))
}
