use super::super::common::{req_artifact_inputs, req_finite_field, req_present, req_u64_at_least};
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

pub(crate) fn validate_benchmark(ev: &Evidence) -> Result<()> {
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

