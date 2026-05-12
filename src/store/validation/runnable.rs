use super::common::{missing_ref, missing_str};
use crate::models::Evidence;
use anyhow::{anyhow, Result};

pub(super) fn validate_prop_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("prop-test requires --ref <path-to-test-file>"));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "prop-test requires --cmd \"<shell cmd that ran the proptest/quickcheck/fuzz>\". the cmd is executed at verify time and the actual exit_code is captured."
        ));
    }
    Ok(())
}

pub(super) fn validate_integration_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("integration-test requires --ref <path-to-test-file>"));
    }
    if missing_str(&ev.target) {
        return Err(anyhow!(
            "integration-test requires --target <url-or-host>. the test must hit a real external system the author does not control; without --target the falsification surface is undocumented."
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "integration-test requires --cmd \"<shell cmd that probed --target>\"."
        ));
    }
    Ok(())
}

pub(super) fn validate_replay_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("replay-test requires --ref <path-to-strategy-or-replay-script>"));
    }
    let dataset = ev.dataset.as_deref().unwrap_or("").trim();
    if dataset.is_empty() {
        return Err(anyhow!(
            "replay-test requires --dataset <path>. the dataset must be real-world capture (not synthetic); it is content-hashed for tamper-evidence."
        ));
    }
    if ev.dataset_hash.is_none() {
        return Err(anyhow!(
            "replay-test requires the dataset at --dataset to exist as a local file at verify time so it can be content-hashed. path '{}' could not be hashed.",
            dataset
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "replay-test requires --cmd \"<shell cmd that replayed --dataset through --ref>\"."
        ));
    }
    Ok(())
}

