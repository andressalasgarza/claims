use super::types::PriorEvidence;
use crate::models::Claim;
use crate::store::detect_drift;
use anyhow::{anyhow, Result};

/// drift-gate: refuse if the local --ref bytes changed since prior evidence.
/// no-op when --acknowledge-drift is set.
pub(super) fn assert_no_drift(claim: &Claim, prior: &PriorEvidence, new_ref_hash: &Option<String>, ack: bool) -> Result<()> {
    if ack {
        return Ok(());
    }
    let Some((prior_hash, prior_at)) = detect_drift(claim, &prior.r#ref, new_ref_hash) else {
        return Ok(());
    };
    Err(anyhow!(
        "test file '{}' has changed since prior evidence:\n  prior hash: {} (recorded {})\n  new hash:   {}\n\nrun with --acknowledge-drift to record this rerun anyway.",
        prior.r#ref,
        &prior_hash[..16.min(prior_hash.len())],
        prior_at,
        new_ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
    ))
}

