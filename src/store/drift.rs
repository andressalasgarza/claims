use super::Store;
use crate::models::{Claim, Evidence, State};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub fn hash_ref_if_local(r: &str) -> Option<String> {
    let p = Path::new(r);
    if p.exists() && p.is_file() {
        let bytes = fs::read(p).ok()?;
        Some(blake3::hash(&bytes).to_hex().to_string())
    } else {
        None
    }
}

/// canonical hash of an Evidence record's *user-supplied* fields. used for
/// dedup at append time. the auto-captured fields (recorded_at, ref_hash,
/// stdout_hash, dataset_hash, cmd_hash) are blanked before hashing so two
/// identical user invocations collapse to the same key, but a rerun that
/// captures a different exit_code stays distinct.
pub fn evidence_user_hash(ev: &Evidence) -> String {
    let mut canon = ev.clone();
    canon.recorded_at = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap_or_else(chrono::Utc::now);
    canon.ref_hash = None;
    canon.stdout_hash = None;
    canon.dataset_hash = None;
    canon.cmd_hash = None;
    let s = serde_json::to_string(&canon).unwrap_or_default();
    blake3::hash(s.as_bytes()).to_hex().to_string()
}

/// returns Some((prior_hash, prior_recorded_at_rfc3339)) if `claim` already has
/// evidence pointing at the same ref path with a different content hash.
pub fn detect_drift(
    claim: &Claim,
    new_ref: &str,
    new_hash: &Option<String>,
) -> Option<(String, String)> {
    let new_hash = new_hash.as_deref()?;
    for prior in &claim.evidence {
        if prior.r#ref != new_ref {
            continue;
        }
        if let Some(prior_hash) = &prior.ref_hash {
            if prior_hash != new_hash {
                return Some((prior_hash.clone(), prior.recorded_at.to_rfc3339()));
            }
        }
    }
    None
}

/// returns Some((prior_ref, prior_recorded_at_rfc3339)) if `claim` has prior
/// evidence whose ref_hash equals the new ref_hash but whose ref *string*
/// differs. this is the rename-pattern dodge: agent copies test.py to
/// test_v2.py (or moves it, or symlinks), then verifies the new path as if it
/// were a distinct piece of evidence. without this check, dedup (which keys on
/// ref string) accepts the duplicate-content-under-different-name as
/// independent evidence, inflating the evidence count for free.
///
/// pairs with detect_drift (same ref + different hash) and detect_dataset_drift
/// (same dataset path + different hash). together they close the rename and
/// content-swap branches of evidence-laundering.
pub fn detect_rename(
    claim: &Claim,
    new_ref: &str,
    new_hash: &Option<String>,
) -> Option<(String, String)> {
    let new_hash = new_hash.as_deref()?;
    for prior in &claim.evidence {
        if prior.r#ref == new_ref {
            continue; // same ref → handled by detect_drift
        }
        if let Some(prior_hash) = &prior.ref_hash {
            if prior_hash == new_hash {
                return Some((prior.r#ref.clone(), prior.recorded_at.to_rfc3339()));
            }
        }
    }
    None
}

/// returns Some((prior_hash, prior_recorded_at_rfc3339)) if `claim` already has
/// replay-test evidence with the same `dataset` path but a different content
/// hash. without this check, an agent can swap the bytes of a parquet file
/// (or any other dataset) between two `replay-test` verifies and clms records
/// both as if the underlying data hadn't changed. detect_drift only catches
/// drift on the ref string; this catches drift on the dataset payload.
pub fn detect_dataset_drift(
    claim: &Claim,
    new_dataset: Option<&str>,
    new_dataset_hash: &Option<String>,
) -> Option<(String, String, String)> {
    let new_dataset = new_dataset?;
    let new_hash = new_dataset_hash.as_deref()?;
    for prior in &claim.evidence {
        let prior_ds = match prior.dataset.as_deref() {
            Some(s) if s == new_dataset => s,
            _ => continue,
        };
        if let Some(prior_hash) = &prior.dataset_hash {
            if prior_hash != new_hash {
                return Some((
                    prior_ds.to_string(),
                    prior_hash.clone(),
                    prior.recorded_at.to_rfc3339(),
                ));
            }
        }
    }
    None
}

/// run a shell command, return (exit_code, stdout_bytes).
pub fn run_cmd(cmd: &str) -> Result<(i32, Vec<u8>)> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("failed to execute: {}", cmd))?;
    Ok((out.status.code().unwrap_or(-1), out.stdout))
}

/// find the latest evidence on a claim that has a stored cmd. used by `rerun`.
/// any method that reports `is_runnable() == true` qualifies.
pub fn latest_runnable_evidence(claim: &Claim) -> Option<&Evidence> {
    claim
        .evidence
        .iter()
        .rev()
        .find(|e| e.cmd.is_some() && e.method.is_runnable())
}

/// flip transitive dependents of `seq` from Verified|Pending to Suspect.
/// returns the seqs that were actually flipped (not all dependents). callers
/// rely on the returned vec to render an honest cascade message; previously
/// this returned every dependent regardless of whether it changed state, which
/// caused `clms refute --cascade` to lie:
///
///   cascade: 1 dependent claim(s) auto-flagged suspect: [42]
///
/// where claim 42 was Unverifiable and stayed Unverifiable. the message
/// claimed an action that did not happen. now the message reports only
/// claims whose state actually changed.
pub fn cascade_suspect(store: &mut Store, seq: u64) -> Result<Vec<u64>> {
    let dependents = store.transitive_dependents(seq)?;
    let mut flipped = Vec::new();
    for dep in &dependents {
        let mut c = store.read_claim(*dep)?;
        if matches!(c.state, State::Verified | State::Pending) {
            c.state = State::Suspect;
            c.updated_at = chrono::Utc::now();
            store.write_claim(&mut c)?;
            flipped.push(*dep);
        }
    }
    Ok(flipped)
}

