use crate::models::{Claim, Evidence, METHODS};
use crate::store::{detect_dataset_drift, detect_drift, detect_rename, evidence_user_hash};
use anyhow::{anyhow, Result};

/// the three drift gates: ref content changed, ref renamed (same hash, new
/// path), dataset bytes changed. all three short-circuit unless
/// --acknowledge-drift is set. each error string is intentionally distinct
/// so the user knows which gate fired.
fn verify_check_drifts(claim: &Claim, ev: &Evidence, ack: bool, seq: u64) -> Result<()> {
    if let Some((prior_hash, prior_at)) = detect_drift(claim, &ev.r#ref, &ev.ref_hash) {
        if !ack {
            return Err(anyhow!(
                "evidence drift detected on '{}':\n  prior hash: {} (recorded {})\n  new hash:   {}\n\n\
the file behind this --ref has changed since prior evidence on this claim. \
if this iteration is intentional, re-run with --acknowledge-drift. if the \
result contradicts the prior conclusion, you should write a new claim and \
refute this one instead.",
                ev.r#ref,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    if let Some((prior_ref, prior_at)) = detect_rename(claim, &ev.r#ref, &ev.ref_hash) {
        if !ack {
            return Err(anyhow!(
                "ref rename detected: --ref '{}' has the same content_hash as prior evidence on claim #{} which used '{}' (recorded {}).\n\n\
the bytes are identical — this is a rename/copy/symlink of the same file under a new path, not new evidence. without this check the dedup (which keys on the ref *string*) would accept this as independent evidence and inflate the count for free. use the original ref, re-run with --acknowledge-drift if the renamed copy is intentional documentation, or write a new claim if it really refers to a distinct file.",
                ev.r#ref, seq, prior_ref, prior_at
            ));
        }
    }
    if let Some((ds, prior_hash, prior_at)) =
        detect_dataset_drift(claim, ev.dataset.as_deref(), &ev.dataset_hash)
    {
        if !ack {
            return Err(anyhow!(
                "dataset drift detected on '{}':\n  prior dataset_hash: {} (recorded {})\n  new dataset_hash:   {}\n\n\
the replay dataset bytes have changed since prior evidence on this claim, even though the dataset path is unchanged. \
an agent that swaps parquet/csv contents can defeat ref-string drift detection. if this iteration is intentional, \
re-run with --acknowledge-drift. if the new bytes contradict the prior conclusion, write a new claim and refute \
this one instead.",
                ds,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.dataset_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    Ok(())
}

/// dedup gate: refuse if the user-supplied evidence fields collide with an
/// existing entry. without this an agent can stack 5x identical verify
/// calls and inflate the apparent evidence count for free.
fn verify_check_dedup(claim: &Claim, ev: &Evidence, seq: u64) -> Result<()> {
    let new_user_hash = evidence_user_hash(ev);
    if claim.evidence.iter().any(|prior| evidence_user_hash(prior) == new_user_hash) {
        return Err(anyhow!(
            "duplicate evidence: claim #{} already has an entry with the same method, ref, and parameters. add new information (a different ref, exit_code, target, dataset, etc.) or write a new claim.",
            seq
        ));
    }
    Ok(())
}

/// min_tier gate: if the claim was stamped with a tier floor at write time
/// (orchestrator intent: "this is a science claim, demand empirical"),
/// refuse the verify if the method's tier is below the floor. uses the
/// METHODS table for tier lookup so it cannot drift from validation logic.
fn verify_check_min_tier(claim: &Claim, ev: &Evidence, seq: u64) -> Result<()> {
    let Some(floor) = claim.min_tier else { return Ok(()); };
    let ev_tier = ev.method.tier();
    if ev_tier.is_at_least(floor) {
        return Ok(());
    }
    let allowed: Vec<&str> = METHODS
        .iter()
        .filter(|m| m.tier.is_at_least(floor))
        .map(|m| m.name)
        .collect();
    Err(anyhow!(
        "claim #{} requires min_tier={} (got method '{}' at tier {}). allowed methods: {}.\n\nthis claim was stamped with a tier floor at creation time (likely by an orchestrator declaring it a science claim). either supply evidence at or above the floor, or write a different claim if the lower-tier evidence is the actual finding.",
        seq,
        floor.as_str(),
        ev.method.as_str(),
        ev_tier.as_str(),
        allowed.join(" | "),
    ))
}

/// run all three semantic gates (drift / dedup / min-tier) as a single
/// caller decision. each gate is independently testable but cmd_verify
/// doesn't care which one fires.
pub(super) fn verify_run_gates(claim: &Claim, ev: &Evidence, ack_drift: bool, seq: u64) -> Result<()> {
    verify_check_drifts(claim, ev, ack_drift, seq)?;
    verify_check_dedup(claim, ev, seq)?;
    verify_check_min_tier(claim, ev, seq)?;
    Ok(())
}

