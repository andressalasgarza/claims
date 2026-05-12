use super::types::PriorEvidence;
use crate::commands::util::assert_not_refuted_rerun;
use crate::models::{Claim, METHODS};
use crate::store::{latest_runnable_evidence, Store};
use anyhow::{anyhow, Result};

/// open the claim, refuse refuted state, snapshot the prior runnable
/// evidence, and tamper-check the cmd_hash. one entry point, three
/// returns: (claim, prior, recomputed_cmd_hash).
pub(super) fn open_and_verify_prior(
    store: &Store, id: &str,
) -> Result<(u64, Claim, PriorEvidence, String)> {
    let seq = store.resolve(id)?;
    let claim = store.read_claim(seq)?;
    assert_not_refuted_rerun(&claim, seq)?;
    let prior = load_runnable_prior(&claim, seq)?;
    let recomputed_cmd_hash = blake3::hash(prior.cmd.as_bytes()).to_hex().to_string();
    assert_cmd_untampered(&prior, &recomputed_cmd_hash, seq)?;
    Ok((seq, claim, prior, recomputed_cmd_hash))
}

/// locate the most recent runnable evidence on the claim and snapshot
/// the fields cmd_rerun needs. errors with a method-list hint when nothing
/// runnable exists.
fn load_runnable_prior(claim: &Claim, seq: u64) -> Result<PriorEvidence> {
    let prior = latest_runnable_evidence(claim).ok_or_else(|| {
        let runnable_names: Vec<&str> = METHODS
            .iter()
            .filter(|m| m.runnable)
            .map(|m| m.name)
            .collect();
        anyhow!(
            "claim #{} has no runnable evidence (need one of {} with stored --cmd)",
            seq,
            runnable_names.join(", ")
        )
    })?;
    Ok(PriorEvidence {
        cmd: prior.cmd.clone().unwrap(),
        r#ref: prior.r#ref.clone(),
        method: prior.method,
        target: prior.target.clone(),
        dataset: prior.dataset.clone(),
        threshold: prior.threshold,
        exit_code: prior.exit_code,
        cmd_hash: prior.cmd_hash.clone(),
    })
}

/// tamper-gate: re-hash the stored cmd and refuse to execute if the json
/// was edited since verify time. without this check, anyone who can write
/// to a .claims/*.json gets arbitrary code execution via `clms rerun`.
fn assert_cmd_untampered(prior: &PriorEvidence, recomputed_hash: &str, seq: u64) -> Result<()> {
    match prior.cmd_hash.as_deref() {
        None => Err(anyhow!(
            "claim #{} has runnable evidence with no cmd_hash (pre-tamper-gate format). \
 re-verify with the current binary to record cmd_hash, then rerun.",
            seq
        )),
        Some(prior_hash) if prior_hash != recomputed_hash => Err(anyhow!(
            "refusing to rerun: cmd has been tampered since verify time.\n  expected cmd_hash: {}\n  actual cmd_hash:   {}\n  cmd:               {:?}\n\nthe `cmd` field in the on-disk evidence record was edited after verify. \
rerun would execute attacker-controlled shell. write a new claim instead.",
            &prior_hash[..16.min(prior_hash.len())],
            &recomputed_hash[..16],
            prior.cmd,
        )),
        _ => Ok(()),
    }
}

