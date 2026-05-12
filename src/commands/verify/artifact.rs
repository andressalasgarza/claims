//! artifact-driven empirical evidence: read the local json file at --ref,
//! extract method-specific fields, and (optionally) echo-check them against
//! cli flags. one trait + two generics replace what used to be six near-
//! identical helper fns (artifact_req_str/_f64/_u64 + maybe_match_*).

use anyhow::{anyhow, Result};

use crate::models::{BenchmarkMetric, DataSource, Estimator, Evidence, EvidenceMethod, HypothesisTest};

/// json field extraction trait. one impl per leaf type we read from
/// artifact files; keeps `artifact_req` / `match_eq` generic without
/// hand-rolling three copies of each.
trait ArtifactField: Sized + PartialEq + std::fmt::Display {
    /// human-readable label for the missing-field error string. preserved
    /// verbatim across the original artifact_req_{str,f64,u64} trio so the
    /// golden error tests keep matching.
    const KIND: &'static str;
    fn extract(v: &serde_json::Value) -> Option<Self>;
}

impl ArtifactField for String {
    const KIND: &'static str = "string";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_str().map(str::to_string) }
}

impl ArtifactField for f64 {
    const KIND: &'static str = "numeric";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_f64() }
}

impl ArtifactField for u64 {
    const KIND: &'static str = "integer";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_u64() }
}

fn artifact_req<T: ArtifactField>(
    value: &serde_json::Value,
    method: EvidenceMethod,
    r#ref: &str,
    key: &str,
) -> Result<T> {
    T::extract(&value[key]).ok_or_else(|| {
        anyhow!(
            "{} artifact '{}' is missing {} field '{}'.",
            method.as_str(),
            r#ref,
            T::KIND,
            key,
        )
    })
}

/// echo-check: if the cli supplied a value, refuse on mismatch with the
/// artifact. the artifact is always the source of truth.
fn match_eq<T: ArtifactField>(
    method: EvidenceMethod,
    r#ref: &str,
    flag: &str,
    cli: Option<T>,
    artifact: &T,
) -> Result<()> {
    if let Some(cli) = cli {
        if &cli != artifact {
            return Err(anyhow!(
                "{} artifact '{}' disagrees with --{}: cli gave '{}', artifact says '{}'. clms now treats the artifact as the source of truth; fix the script or drop the stale flag.",
                method.as_str(),
                r#ref,
                flag,
                cli,
                artifact,
            ));
        }
    }
    Ok(())
}

fn read_local_json_artifact(method: EvidenceMethod, r#ref: &str) -> Result<serde_json::Value> {
    let path = std::path::Path::new(r#ref);
    if !path.is_file() {
        return Err(anyhow!(
            "{} requires --ref to be a local file produced by --cmd. '{}' does not exist as a local file after execution.",
            method.as_str(),
            r#ref,
        ));
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("read {} artifact '{}': {}", method.as_str(), r#ref, e))?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        anyhow!(
            "{} --ref '{}' must contain valid JSON so clms can derive the measured values from the artifact. {}",
            method.as_str(),
            r#ref,
            e,
        )
    })?;
    if !value.is_object() {
        return Err(anyhow!(
            "{} --ref '{}' must contain a top-level JSON object.",
            method.as_str(),
            r#ref,
        ));
    }
    Ok(value)
}

fn populate_stat_test_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let test_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "test_type")?;
    let parsed_test = HypothesisTest::parse(&test_name)?;
    let p: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "p_value")?;
    let n: u64 = artifact_req(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(&src_name)?;

    match_eq(ev.method, &ev.r#ref, "test-type", ev.test_type.map(|t| t.as_str().to_string()), &parsed_test.as_str().to_string())?;
    match_eq(ev.method, &ev.r#ref, "p-value", ev.p_value, &p)?;
    match_eq(ev.method, &ev.r#ref, "sample-size", ev.sample_size, &n)?;
    match_eq(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str().to_string()), &parsed_src.as_str().to_string())?;

    ev.test_type = Some(parsed_test);
    ev.p_value = Some(p);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

fn populate_benchmark_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let metric_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "metric")?;
    let parsed_metric = BenchmarkMetric::parse(&metric_name)?;
    let metric_value: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "metric_value")?;
    let n: u64 = artifact_req(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(&src_name)?;

    match_eq(ev.method, &ev.r#ref, "metric", ev.metric.map(|m| m.as_str().to_string()), &parsed_metric.as_str().to_string())?;
    match_eq(ev.method, &ev.r#ref, "metric-value", ev.metric_value, &metric_value)?;
    match_eq(ev.method, &ev.r#ref, "sample-size", ev.sample_size, &n)?;
    match_eq(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str().to_string()), &parsed_src.as_str().to_string())?;

    ev.metric = Some(parsed_metric);
    ev.metric_value = Some(metric_value);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

fn populate_estimate_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let estimator_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "estimator")?;
    let parsed_estimator = Estimator::parse(&estimator_name)?;
    let point: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "point_value")?;
    let lo: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "ci_lower")?;
    let hi: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "ci_upper")?;
    let conf: f64 = artifact_req(&doc, ev.method, &ev.r#ref, "confidence_level")?;
    let n: u64 = artifact_req(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name: String = artifact_req(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(&src_name)?;

    match_eq(ev.method, &ev.r#ref, "estimator", ev.estimator.map(|m| m.as_str().to_string()), &parsed_estimator.as_str().to_string())?;
    match_eq(ev.method, &ev.r#ref, "point-value", ev.point_value, &point)?;
    match_eq(ev.method, &ev.r#ref, "ci-lower", ev.ci_lower, &lo)?;
    match_eq(ev.method, &ev.r#ref, "ci-upper", ev.ci_upper, &hi)?;
    match_eq(ev.method, &ev.r#ref, "confidence-level", ev.confidence_level, &conf)?;
    match_eq(ev.method, &ev.r#ref, "sample-size", ev.sample_size, &n)?;
    match_eq(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str().to_string()), &parsed_src.as_str().to_string())?;

    ev.estimator = Some(parsed_estimator);
    ev.point_value = Some(point);
    ev.ci_lower = Some(lo);
    ev.ci_upper = Some(hi);
    ev.confidence_level = Some(conf);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

pub(crate) fn populate_empirical_artifact_fields(ev: &mut Evidence) -> Result<()> {
    match ev.method {
        EvidenceMethod::StatTest => populate_stat_test_from_artifact(ev),
        EvidenceMethod::Benchmark => populate_benchmark_from_artifact(ev),
        EvidenceMethod::Estimate => populate_estimate_from_artifact(ev),
        _ => Ok(()),
    }
}

/// shared exit-code gate for artifact-driven methods. both cmd_verify and
/// cmd_rerun must refuse non-zero exits because the json artifact at --ref
/// is only trustworthy when --cmd succeeded.
pub(crate) fn assert_artifact_cmd_zero(
    method: EvidenceMethod,
    cmd: &str,
    exit: i32,
    stdout: &[u8],
    ctx: &str,
) -> Result<()> {
    if exit == 0 {
        return Ok(());
    }
    let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
    let ellipsis = if stdout.len() > 200 { " ..." } else { "" };
    Err(anyhow!(
        "{}: {} --cmd exited {} (expected 0 so clms can parse the local artifact at --ref).\n  cmd:    {}\n  stdout: {:?}{}",
        ctx,
        method.as_str(),
        exit,
        cmd,
        preview,
        ellipsis,
    ))
}

pub(crate) fn preflight_artifact_driven_inputs(ev: &Evidence) -> Result<()> {
    let cmd = ev.cmd.as_deref().unwrap_or("").trim();
    if ev.r#ref.trim().is_empty() {
        return Err(anyhow!(
            "{} requires --ref <path-to-local-json-artifact>",
            ev.method.as_str()
        ));
    }
    if cmd.is_empty() {
        return Err(anyhow!(
            "{} requires --cmd \"<shell cmd that produces the local json artifact at --ref>\".",
            ev.method.as_str()
        ));
    }
    Ok(())
}
