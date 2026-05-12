use crate::commands::verify::artifact::{
    assert_artifact_cmd_zero, populate_empirical_artifact_fields,
};
use crate::models::{Evidence, EvidenceMethod};
use crate::store::{validate_evidence, Store};
use anyhow::Result;

/// for artifact-driven methods, assert exit==0, populate empirical fields
/// from the json at --ref, then validate. for other methods, no-op.
pub(super) fn maybe_validate_artifact_evidence(
    ev: &mut Evidence, store: &Store, exit_code: i32, stdout: &[u8], seq: u64,
) -> Result<()> {
    if !matches!(
        ev.method,
        EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate
    ) {
        return Ok(());
    }
    assert_artifact_cmd_zero(
        ev.method,
        ev.cmd.as_deref().unwrap_or(""),
        exit_code,
        stdout,
        &format!("rerun on claim #{}", seq),
    )?;
    populate_empirical_artifact_fields(ev)?;
    validate_evidence(ev, store, seq)
}

