use super::super::AGENT;
use crate::models::{
    ArchaeologyMeta, Claim, StakeSignal as ModelStakeSignal,
    SuggestedEvidence as ModelSuggestedEvidence, State, SCHEMA_VERSION as CLAIM_SCHEMA_VERSION,
};
use crate::store::Store;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::Path;
use ulid::Ulid;

pub(super) fn build_pending_claim(
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

