use super::{req_artifact_inputs, req_finite_field, req_present, req_u64_at_least};
use crate::models::{BenchmarkMetric, Evidence};
use anyhow::{anyhow, Result};

/// direction-aware threshold check for benchmark. extracted to keep the
/// main validator flat: this captures the (metric, value, threshold) triple
/// + the higher/lower-better routing as a single decision in the caller.
fn benchmark_check_threshold(metric: BenchmarkMetric, v: f64, t: f64) -> Result<()> {
    let passes = if metric.is_higher_better() { v >= t } else { v <= t };
    if passes {
        return Ok(());
    }
    let dir = if metric.is_higher_better() {
        "higher-better, need value >= threshold"
    } else {
        "lower-better, need value <= threshold"
    };
    Err(anyhow!(
        "benchmark refused: metric_value={} did not meet threshold={} for metric '{}' ({}).\nstate stays pending; either fix the eval, lower the threshold honestly, or write a different claim if the miss is the actual finding.",
        v, t, metric.as_str(), dir,
    ))
}

/// pull required numeric/enum fields from a benchmark evidence record.
/// returns the validated (metric, metric_value, threshold) triple. extracted
/// to keep validate_benchmark's body flat; closures inside this fn count
/// toward this fn's ccn instead of the parent's.
fn benchmark_collect_required(ev: &Evidence) -> Result<(BenchmarkMetric, f64, f64)> {
    let metric = req_present(ev.metric, || anyhow!(
        "benchmark artifact missing 'metric'. valid: auc-roc | auc-pr | f1 | precision | recall | accuracy | balanced-accuracy | mcc | kappa-cohen | r2 | log-loss | brier | rmse | mae | mape."
    ))?;
    let v = req_finite_field(
        ev.metric_value,
        || anyhow!("benchmark artifact missing 'metric_value'"),
        |x| anyhow!(
            "benchmark --metric-value must be finite (got {}). NaN/inf are not measurable values.",
            x
        ),
    )?;
    let t = req_finite_field(
        ev.threshold,
        || anyhow!("benchmark requires --threshold <float>"),
        |x| anyhow!(
            "benchmark --threshold must be finite (got {}). NaN/inf are not measurable thresholds.",
            x
        ),
    )?;
    Ok((metric, v, t))
}

/// sample size + data-source presence for benchmark. extracted for the
/// same reason as benchmark_collect_required.
fn benchmark_check_meta(ev: &Evidence) -> Result<()> {
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("benchmark artifact missing 'sample_size'"),
        |n| anyhow!(
            "benchmark --sample-size must be >= 2 (got {}). a metric computed on one sample is not measurable evidence.",
            n
        ),
    )?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "benchmark artifact missing 'data_source' (expected real|live). synthetic eval sets cannot falsify a claim about real-world performance."
    ))?;
    Ok(())
}

pub(super) fn validate_benchmark(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "benchmark requires --ref <path-to-local-json-artifact>",
        "benchmark requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "benchmark requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time, hashes stdout, then parses metric/sample metadata from the artifact instead of trusting cli flags.",
    )?;
    let (metric, v, t) = benchmark_collect_required(ev)?;
    benchmark_check_meta(ev)?;
    benchmark_check_threshold(metric, v, t)
}

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

pub(super) fn validate_estimate(ev: &Evidence) -> Result<()> {
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

pub(super) fn validate_stat_test(ev: &Evidence) -> Result<()> {
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

