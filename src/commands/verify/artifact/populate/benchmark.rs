use super::fields::{artifact_req, match_eq, read_local_json_artifact};
use crate::models::{BenchmarkMetric, DataSource, Evidence};
use anyhow::Result;

pub(crate) fn populate_benchmark_from_artifact(ev: &mut Evidence) -> Result<()> {
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

