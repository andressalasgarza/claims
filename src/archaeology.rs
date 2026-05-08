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

use crate::models::{
    ArchaeologyMeta, Claim, StakeSignal as ModelStakeSignal,
    SuggestedEvidence as ModelSuggestedEvidence, State, SCHEMA_VERSION as CLAIM_SCHEMA_VERSION,
};
use crate::git;
use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use ulid::Ulid;

pub const AGENT: &str = "archaeology";
pub const SCHEMA_VERSION: &str = "archaeology/v2";
pub const DEFAULT_MAX: usize = 10;
pub const MAX_CEILING: usize = 50;

const CLAIM_MARKERS: &[&str] = &["// clms-claim:", "# clms-claim:"];
const EVIDENCE_MARKERS: &[&str] = &["// clms-evidence:", "# clms-evidence:"];

// source code only. markdown explicitly excluded — docs use `// clms-claim:`
// in code-block examples to demonstrate the syntax, but those are
// illustrations, not assertions. annotations belong in actual source.
const SOURCE_EXTS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "go"];
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

pub fn dispatch(sub: Sub, store: &mut Store, fmt: OutputFormat) -> Result<()> {
    match sub {
        Sub::Suggest(a) => cmd_suggest(a, fmt),
        Sub::Commit(a) => cmd_commit(store, a, fmt),
        Sub::Purge(a) => cmd_purge(store, a, fmt),
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
    /// chronological provenance. `git blame` author-time for in-source
    /// annotations; manifest mtime for proposals; None if neither resolves
    /// (uncommitted line, missing repo, etc.). harvester sorts by this
    /// ascending so older claims come first — they've earned their stake
    /// by surviving subsequent edits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
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

    let (mut all, proposals_count) = harvest_all(&root)?;
    let actual = all.len();
    let truncated = actual.saturating_sub(max);
    all.truncate(max);

    let payload = build_suggest_payload(
        &all, max, actual, truncated, &sources_enabled, proposals_count,
    );
    emit_suggest_output(&payload, fmt, args.output, actual, truncated)
}

/// runs both intent surfaces (in-source annotations + proposals.json),
/// dedups by candidate_id, and sorts chronologically (oldest first).
///
/// chronological-first is the staking mechanism: claims earn stake by
/// surviving subsequent edits. an annotation that has lived through three
/// years of refactors has demonstrated stake that yesterday's hasn't.
/// candidates without a resolvable timestamp (uncommitted, missing repo)
/// sort to the end as "effectively newest." file:line is the tiebreak.
///
/// returns (all_candidates, proposals_count_after_dedup).
fn harvest_all(root: &Path) -> Result<(Vec<Candidate>, usize)> {
    let mut all = harvest_annotations(root)?;
    let proposals = harvest_proposals(root)?;
    let proposals_added = proposals.len();
    all.extend(proposals);

    // dedup by candidate_id (proposal can shadow an existing source annotation
    // if both encode the same kind|text|where; keep first occurrence)
    let mut seen: HashSet<String> = HashSet::new();
    all.retain(|c| seen.insert(c.id.clone()));

    all.sort_by(|a, b| candidate_chronological_order(a, b));

    Ok((all, proposals_added))
}

fn candidate_chronological_order(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    let tiebreak = (&a.source_meta.file, a.source_meta.line)
        .cmp(&(&b.source_meta.file, b.source_meta.line));
    match (&a.created_at, &b.created_at) {
        (Some(x), Some(y)) => x.cmp(y).then(tiebreak),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => tiebreak,
    }
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

// agent intent surface. parallel to in-source `// clms-claim:` annotations.
// schema: { "version": "archaeology/v2", "proposals": [<row>, ...] }
// row:    { "text": str, "where": str, "snippet"?: str, "suggested_evidence"?: [...] }
//
// candidates emitted from this surface use the same kind (`clms-claim-annotation`)
// as in-source annotations, so the judge agent sees them identically. distinguish
// origin via source_meta.file (== ".archaeology/proposals.json").
fn harvest_proposals(root: &Path) -> Result<Vec<Candidate>> {
    let path = root.join(".archaeology/proposals.json");
    let Some(arr) = load_proposals_manifest(&path)? else {
        return Ok(Vec::new());
    };
    let manifest_mtime = git::file_mtime(&path);
    let mut out = Vec::new();
    for (idx, row) in arr.iter().enumerate() {
        if let Some(c) = parse_proposal_row(&path, idx, row, manifest_mtime)? {
            out.push(c);
        }
    }
    Ok(out)
}

/// reads + parses + version-checks proposals.json. returns Ok(None) if the
/// manifest does not exist (the cold-start case is benign).
fn load_proposals_manifest(path: &Path) -> Result<Option<Vec<serde_json::Value>>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
    let version = v["version"].as_str().unwrap_or("");
    if version != SCHEMA_VERSION {
        bail!(
            "{}: expected version '{}', got '{}'",
            path.display(), SCHEMA_VERSION, version
        );
    }
    let arr = v["proposals"]
        .as_array()
        .ok_or_else(|| anyhow!("{}: 'proposals' must be an array", path.display()))?
        .clone();
    Ok(Some(arr))
}

/// parses one row from proposals.json. returns Ok(None) for empty-text rows
/// (silently skipped, not an error).
fn parse_proposal_row(
    path: &Path,
    idx: usize,
    row: &serde_json::Value,
    manifest_mtime: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Option<Candidate>> {
    let text = row["text"]
        .as_str()
        .ok_or_else(|| anyhow!("{}: proposals[{}].text missing", path.display(), idx))?
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let where_str = row["where"]
        .as_str()
        .ok_or_else(|| anyhow!("{}: proposals[{}].where missing", path.display(), idx))?
        .to_string();
    let snippet = row["snippet"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("// (proposal) clms-claim: {}", text));
    let suggested_evidence = parse_suggested_evidence(row.get("suggested_evidence"));
    let id = candidate_id("clms-claim-annotation", &text, &where_str);
    // proposals don't live in git history; date them by manifest mtime so
    // agent proposals sit on a comparable timeline w/ source-blame timestamps.
    Ok(Some(Candidate {
        id,
        kind: "clms-claim-annotation".into(),
        text,
        stake_signal: StakeSignal { r#where: where_str, snippet },
        suggested_evidence,
        source_meta: SourceMeta {
            file: ".archaeology/proposals.json".to_string(),
            line: (idx as u32) + 1,
        },
        created_at: manifest_mtime,
        debate: None,
    }))
}

fn parse_suggested_evidence(arr: Option<&serde_json::Value>) -> Vec<SuggestedEvidence> {
    let Some(items) = arr.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|ev| {
            let method = ev["method"].as_str()?.trim();
            if method.is_empty() {
                return None;
            }
            Some(SuggestedEvidence {
                method: method.to_string(),
                cmd: ev["cmd"].as_str().map(String::from),
                r#ref: ev["ref"].as_str().map(String::from),
                note: ev["note"]
                    .as_str()
                    .unwrap_or("advisory only; not run by archaeology. promotion via `clms verify`")
                    .to_string(),
            })
        })
        .collect()
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
            let created_at = git::blame_line_time(path, (idx as u32) + 1);
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
                created_at,
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
    // default to prop-test for un-promoted candidates: it has the lowest
    // setup cost among empirical methods (no --target, no --dataset) and is
    // still falsificatory. integration-test and replay-test require
    // explicit values that the proposer cannot pick on the agent's behalf.
    let mut method = "prop-test".to_string();
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

// ---------- commit (phase 3) ----------

fn cmd_commit(store: &mut Store, args: CommitArgs, fmt: OutputFormat) -> Result<()> {
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

/// reads survivors.json from disk, parses it, and runs `validate_plan`.
fn load_and_validate_plan(path: &Path, keep: usize) -> Result<serde_json::Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read survivors plan {}", path.display()))?;
    let plan: serde_json::Value =
        serde_json::from_str(&raw).context("survivors.json is not valid JSON")?;
    validate_plan(&plan, keep)?;
    Ok(plan)
}

struct CommitOutcome {
    written: usize,
    skipped: usize,
    summary: Vec<serde_json::Value>,
}

/// iterates kept candidates, builds + writes pending claims, accumulates a
/// per-claim summary. idempotent on candidate_id collision.
fn process_candidates(
    store: &mut Store,
    candidates: &[serde_json::Value],
    session: &str,
    archaeology_dir: &Path,
    existing_ids: &HashSet<String>,
) -> Result<CommitOutcome> {
    let mut out = CommitOutcome { written: 0, skipped: 0, summary: Vec::new() };
    for c in candidates {
        if !c["keep"].as_bool().unwrap_or(false) {
            continue;
        }
        let cid = c["id"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate missing id"))?
            .to_string();
        if existing_ids.contains(&cid) {
            out.skipped += 1;
            continue;
        }
        let claim = build_pending_claim(store, c, session, archaeology_dir)?;
        store.write_claim(&mut claim_mut(claim))?;
        out.written += 1;
        out.summary.push(serde_json::json!({
            "candidate_id": cid,
            "text": c["text"],
        }));
    }
    Ok(out)
}

fn emit_commit_report(
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

fn validate_plan(plan: &serde_json::Value, keep_cap: usize) -> Result<()> {
    let v = plan["version"].as_str().unwrap_or("");
    if v != SCHEMA_VERSION {
        bail!(
            "survivors.json: expected version '{}', got '{}'",
            SCHEMA_VERSION, v
        );
    }
    let candidates = plan["candidates"]
        .as_array()
        .ok_or_else(|| anyhow!("survivors.json: 'candidates' must be an array"))?;
    let mut keep_count = 0usize;
    for (idx, c) in candidates.iter().enumerate() {
        let cid = c["id"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate[{}] missing 'id'", idx))?;
        let keep = c["keep"].as_bool();
        if keep.is_none() {
            bail!("candidate {} missing 'keep' boolean", cid);
        }
        if c["debate"].is_null() {
            bail!(
                "candidate {} has debate=null. archaeology commit requires curated survivors. \
                 run the debate phase first (see docs/archaeology.md phase 2).",
                cid
            );
        }
        let verdict = c["debate"]["judge"]["verdict"].as_str();
        if verdict.is_none() {
            bail!(
                "candidate {} missing debate.judge.verdict (must be \"keep\" or \"drop\")",
                cid
            );
        }
        if keep.unwrap_or(false) {
            if verdict != Some("keep") {
                bail!(
                    "candidate {} has keep:true but debate.judge.verdict={:?}",
                    cid, verdict
                );
            }
            keep_count += 1;
        }
    }
    if keep_count > keep_cap {
        bail!(
            "survivors.json has {} keep:true rows, exceeds --keep {}. tighten the judge or raise the cap",
            keep_count, keep_cap
        );
    }
    Ok(())
}

fn scan_existing_candidate_ids(store: &Store) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for seq in store.all_seqs()? {
        let claim = match store.read_claim(seq) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(meta) = &claim.archaeology_meta {
            out.insert(meta.candidate_id.clone());
        }
    }
    Ok(out)
}

fn build_pending_claim(
    store: &mut Store,
    c: &serde_json::Value,
    session: &str,
    archaeology_dir: &Path,
) -> Result<Claim> {
    let cid = c["id"].as_str().unwrap_or("").to_string();
    let text = c["text"].as_str().unwrap_or("").to_string();
    if text.trim().is_empty() {
        bail!("candidate {} has empty text", cid);
    }
    let kind = c["kind"].as_str().unwrap_or("unknown").to_string();
    let stake_signal = ModelStakeSignal {
        r#where: c["stake_signal"]["where"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        snippet: c["stake_signal"]["snippet"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    };
    let suggested_evidence: Vec<ModelSuggestedEvidence> = c["suggested_evidence"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let method = e["method"].as_str()?.to_string();
                    Some(ModelSuggestedEvidence {
                        method,
                        cmd: e["cmd"].as_str().map(String::from),
                        r#ref: e["ref"].as_str().map(String::from),
                        note: e["note"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let transcript_path = archaeology_dir.join(format!("{}.json", cid));
    fs::write(
        &transcript_path,
        serde_json::to_string_pretty(c).context("serialize transcript")?,
    )
    .with_context(|| format!("write transcript {}", transcript_path.display()))?;

    let judge_rationale = c["debate"]["judge"]["rationale"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let judge_rank = c["debate"]["judge"]["rank"].as_u64().unwrap_or(0) as u32;

    let now = Utc::now();
    Ok(Claim {
        schema_version: CLAIM_SCHEMA_VERSION.into(),
        id: Ulid::new(),
        seq: store.next_seq()?,
        text,
        state: State::Pending,
        tags: vec![],
        evidence: vec![],
        edges: vec![],
        agent: Some(AGENT.to_string()),
        session: Some(session.to_string()),
        git_sha: None,
        created_at: now,
        updated_at: now,
        content_hash: None,
        archaeology_meta: Some(ArchaeologyMeta {
            candidate_id: cid,
            kind,
            stake_signal,
            suggested_evidence,
            debate_transcript_ref: transcript_path.display().to_string(),
            judge_rationale,
            judge_rank,
        }),
    })
}

fn claim_mut(c: Claim) -> Claim {
    c
}

// ---------- purge ----------

fn cmd_purge(store: &mut Store, args: PurgeArgs, fmt: OutputFormat) -> Result<()> {
    let target_agent = args.agent.unwrap_or_else(|| AGENT.to_string());
    let target_session = args.session;

    let mut victims: Vec<u64> = Vec::new();
    for seq in store.all_seqs()? {
        let claim = match store.read_claim(seq) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let agent_match = claim.agent.as_deref() == Some(target_agent.as_str());
        let session_match = claim.session.as_deref() == Some(target_session.as_str());
        if agent_match && session_match {
            victims.push(seq);
        }
    }

    let mut removed = 0usize;
    for seq in &victims {
        let path = PathBuf::from(".claims").join(format!("{:06}.json", seq));
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("remove {}", path.display()))?;
            removed += 1;
        }
    }

    let reindexed = store.reindex_all()?;

    if matches!(fmt, OutputFormat::Ai) {
        println!(
            "{}",
            serde_json::json!({
                "agent": target_agent,
                "session": target_session,
                "matched_seqs": victims,
                "removed_files": removed,
                "reindexed_total": reindexed,
            })
        );
    } else if victims.is_empty() {
        println!(
            "no claims matched agent={} session={}",
            target_agent, target_session
        );
    } else {
        println!(
            "purged {} claims (seqs: {:?}) for agent={} session={}",
            removed, victims, target_agent, target_session
        );
        println!("reindexed {} remaining claims", reindexed);
    }
    Ok(())
}

#[allow(dead_code)]
pub fn _ensure_unused() -> Result<()> {
    Err(anyhow!("placeholder"))
}
