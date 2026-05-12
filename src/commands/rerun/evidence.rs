use super::types::PriorEvidence;
use crate::models::Evidence;
use chrono::Utc;

/// construct the rerun's Evidence record. all method-specific fields
/// (metric_value, point_value, etc.) are None and get populated downstream
/// by populate_empirical_artifact_fields when the method is artifact-driven.
pub(super) fn build_rerun_evidence(
    prior: &PriorEvidence, exit_code: i32, new_ref_hash: Option<String>,
    new_dataset_hash: Option<String>, stdout_hash: Option<String>,
    recomputed_cmd_hash: String, seq: u64,
) -> Evidence {
    Evidence {
        method: prior.method,
        r#ref: prior.r#ref.clone(),
        note: Some(format!("rerun via `clms rerun {}`", seq)),
        p_value: None,
        sample_size: None,
        test_type: None,
        exit_code: Some(exit_code),
        quote: None,
        from_claims: vec![],
        ref_hash: new_ref_hash,
        cmd: Some(prior.cmd.clone()),
        cmd_hash: Some(recomputed_cmd_hash),
        stdout_hash,
        target: prior.target.clone(),
        dataset: prior.dataset.clone(),
        dataset_hash: new_dataset_hash,
        data_source: None,
        metric: None,
        metric_value: None,
        threshold: prior.threshold,
        estimator: None,
        point_value: None,
        ci_lower: None,
        ci_upper: None,
        confidence_level: None,
        recorded_at: Utc::now(),
    }
}

