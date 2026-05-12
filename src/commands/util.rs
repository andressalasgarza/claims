//! cross-command helpers. these used to be free functions in main.rs;
//! pulled here so every `cmd_*` module can share them without re-importing
//! `crate::main`-private items.

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::models::{Claim, State};
use crate::store::Store;

/// shared refused-claim gate for rerun: cmd_rerun must refuse to operate on
/// a refuted claim because re-collecting evidence cannot un-refute it.
pub(crate) fn assert_not_refuted_rerun(claim: &Claim, seq: u64) -> Result<()> {
    if claim.state != State::Refuted {
        return Ok(());
    }
    Err(anyhow!(
        "claim #{} is refuted; rerun is not meaningful. write a new claim.",
        seq
    ))
}

/// CLAIMS_REPAIR=1 disables claim-integrity verification in store::read_claim
/// so a human can recover from a corrupted .claims/ tree (forensic
/// forward-read). any *mutating* command run while repair is in effect would
/// carry the tampered state forward; in the worst case (rerun) it would
/// execute a tampered cmd field as shell. mutators bail at the top with this
/// helper.
pub(crate) fn refuse_in_repair_mode(op: &str) -> Result<()> {
    if std::env::var("CLAIMS_REPAIR").is_ok() {
        return Err(anyhow!(
            "{}: refusing under CLAIMS_REPAIR=1. repair mode is read-only — it skips claim-integrity verification for forensic recovery (show, diff-evidence, timeline, context). fix the corrupted records first, then unset CLAIMS_REPAIR before mutating state. without this guard, a tampered claim could be carried forward into add/verify/refute/rerun/reindex or archaeology mutation.",
            op
        ));
    }
    Ok(())
}

/// shared write-tail used by every mutator that appends to a claim: bump
/// updated_at and persist. callers handle their own rendering since the
/// surrounding output (cascade note on refute, suspect flip on rerun) is
/// method-specific.
pub(crate) fn commit_evidence(claim: &mut Claim, store: &mut Store) -> Result<()> {
    claim.updated_at = Utc::now();
    store.write_claim(claim)
}
