use super::{CommitArgs, AGENT, SCHEMA_VERSION};
use crate::models::{
    ArchaeologyMeta, Claim, StakeSignal as ModelStakeSignal,
    SuggestedEvidence as ModelSuggestedEvidence, State, SCHEMA_VERSION as CLAIM_SCHEMA_VERSION,
};
use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use ulid::Ulid;

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

fn validate_plan_candidate(idx: usize, c: &serde_json::Value) -> Result<bool> {
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
    let keeps = keep.unwrap_or(false);
    if keeps && verdict != Some("keep") {
        bail!(
            "candidate {} has keep:true but debate.judge.verdict={:?}",
            cid, verdict
        );
    }
    Ok(keeps)
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
        if validate_plan_candidate(idx, c)? {
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
        integrity_mac: None,
        archaeology_meta: Some(ArchaeologyMeta {
            candidate_id: cid,
            kind,
            stake_signal,
            suggested_evidence,
            debate_transcript_ref: transcript_path.display().to_string(),
            judge_rationale,
            judge_rank,
        }),
        backfilled: true,
        min_tier: None,
    })
}

fn claim_mut(c: Claim) -> Claim {
    c
}
