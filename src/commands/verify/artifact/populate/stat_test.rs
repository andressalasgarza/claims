use super::fields::{artifact_req, match_eq, read_local_json_artifact};
use crate::models::{DataSource, Evidence, HypothesisTest};
use anyhow::Result;

pub(crate) fn populate_stat_test_from_artifact(ev: &mut Evidence) -> Result<()> {
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

