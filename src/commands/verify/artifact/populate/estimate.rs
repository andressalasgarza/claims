use super::fields::{artifact_req, match_eq, read_local_json_artifact};
use crate::models::{DataSource, Estimator, Evidence};
use anyhow::Result;

pub(crate) fn populate_estimate_from_artifact(ev: &mut Evidence) -> Result<()> {
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

