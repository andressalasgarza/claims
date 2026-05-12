use super::super::common::{req_artifact_inputs, req_finite_field, req_present, req_u64_at_least};
use crate::models::Evidence;
use anyhow::{anyhow, Result};

/// CI shape checks for estimate: lo and hi must be finite, lo ≤ hi, and
/// the point estimate must lie within [lo, hi]. extracted so the parent
/// stays flat. uses three sequential checks; caller pays 1 ccn for the
/// whole CI block.
fn estimate_check_ci(point: f64, lo: f64, hi: f64) -> Result<()> {
    if !lo.is_finite() || !hi.is_finite() {
        return Err(anyhow!(
            "estimate CI bounds must be finite (got ci_lower={}, ci_upper={}).",
            lo, hi
        ));
    }
    if lo > hi {
        return Err(anyhow!(
            "estimate --ci-lower={} > --ci-upper={}. lower bound must be ≤ upper bound.",
            lo, hi
        ));
    }
    if point < lo || point > hi {
        return Err(anyhow!(
            "estimate --point-value={} is outside the declared CI [{}, {}]. the point estimate must lie within its own interval; otherwise the CI is misreported.",
            point, lo, hi
        ));
    }
    Ok(())
}

/// confidence-level must be a probability strictly between 0 and 1.
/// extracted so the parent doesn't accumulate the 3-way conjunction.
fn estimate_check_confidence(conf: f64) -> Result<()> {
    if conf.is_finite() && conf > 0.0 && conf < 1.0 {
        return Ok(());
    }
    Err(anyhow!(
        "estimate --confidence-level must be in (0, 1) (got {}). typical values: 0.90, 0.95, 0.99.",
        conf
    ))
}

/// collect estimator presence + point_value + ci bounds. closures pile up
/// here instead of the parent.
fn estimate_collect_point_and_ci(ev: &Evidence) -> Result<(f64, f64, f64)> {
    req_present(ev.estimator, || anyhow!(
        "estimate artifact missing 'estimator'. valid: mean | median | geometric-mean | std-dev | std-error | variance | skewness | kurtosis | cohens-d | odds-ratio | risk-ratio | correlation | spearman-rho."
    ))?;
    let point = req_finite_field(
        ev.point_value,
        || anyhow!("estimate artifact missing 'point_value'"),
        |x| anyhow!(
            "estimate --point-value must be finite (got {}). NaN/inf are not measurable values.",
            x
        ),
    )?;
    let lo = req_present(ev.ci_lower, || anyhow!("estimate artifact missing 'ci_lower'"))?;
    let hi = req_present(ev.ci_upper, || anyhow!("estimate artifact missing 'ci_upper'"))?;
    Ok((point, lo, hi))
}

/// confidence-level + sample-size + data-source for estimate.
fn estimate_check_meta(ev: &Evidence) -> Result<()> {
    let conf = req_present(
        ev.confidence_level,
        || anyhow!("estimate artifact missing 'confidence_level'"),
    )?;
    estimate_check_confidence(conf)?;
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("estimate artifact missing 'sample_size'"),
        |n| anyhow!(
            "estimate --sample-size must be >= 2 (got {}). a CI from a single sample is not measurable.",
            n
        ),
    )?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "estimate artifact missing 'data_source' (expected real|live). estimators on synthetic data only describe the simulator (circular)."
    ))?;
    Ok(())
}

pub(crate) fn validate_estimate(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "estimate requires --ref <path-to-local-json-artifact>",
        "estimate requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "estimate requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time, hashes stdout, then parses estimator/ci metadata from the artifact instead of trusting cli flags.",
    )?;
    let (point, lo, hi) = estimate_collect_point_and_ci(ev)?;
    estimate_check_ci(point, lo, hi)?;
    estimate_check_meta(ev)
}

