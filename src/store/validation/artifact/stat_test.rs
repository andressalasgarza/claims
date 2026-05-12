use super::super::common::{req_artifact_inputs, req_present, req_u64_at_least};
use crate::models::Evidence;
use anyhow::{anyhow, Result};

/// p_value must be a probability AND not NaN. nan check is separate from
/// the range check because nan compares false to both bounds and would slip
/// through `contains()`.
fn stat_test_check_p_value(p: f64) -> Result<()> {
    if (0.0..=1.0).contains(&p) && !p.is_nan() {
        return Ok(());
    }
    Err(anyhow!(
        "stat-test --p-value must be in [0.0, 1.0] (got {}). a p-value is a probability — outside that range it is not a p-value.",
        p
    ))
}

/// sample-size + test_type + data-source for stat-test. extracted so the
/// closure cost doesn't pile onto validate_stat_test.
fn stat_test_check_meta(ev: &Evidence) -> Result<()> {
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("stat-test artifact missing 'sample_size'"),
        |n| anyhow!(
            "stat-test --sample-size must be >= 2 (got {}). a single sample (or zero) cannot falsify a distributional claim.",
            n
        ),
    )?;
    req_present(ev.test_type, || anyhow!("stat-test artifact missing 'test_type'"))?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "stat-test artifact missing 'data_source' (expected real|live). simulated/synthetic data is refused: it cannot falsify a claim about reality."
    ))?;
    Ok(())
}

pub(crate) fn validate_stat_test(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "stat-test requires --ref <path-to-local-json-artifact>",
        "stat-test requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "stat-test requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time and parses p_value/sample metadata from the artifact instead of trusting cli flags.",
    )?;
    let p = req_present(ev.p_value, || anyhow!("stat-test artifact missing 'p_value'"))?;
    stat_test_check_p_value(p)?;
    stat_test_check_meta(ev)
}

