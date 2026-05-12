use super::types::{PriorEvidence, RerunExec};
use crate::store::{hash_ref_if_local, run_cmd};
use anyhow::Result;

/// execute the prior cmd and hash everything: ref bytes, dataset bytes,
/// stdout, and the (already-recomputed) cmd_hash. one Result so the
/// caller sees a single failure point.
pub(super) fn execute_and_hash(prior: &PriorEvidence, recomputed_cmd_hash: String) -> Result<RerunExec> {
    eprintln!("rerunning: {}", prior.cmd);
    let (exit_code, stdout) = run_cmd(&prior.cmd)?;
    let new_ref_hash = hash_ref_if_local(&prior.r#ref);
    let new_dataset_hash = prior.dataset.as_deref().and_then(hash_ref_if_local);
    let stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
    Ok(RerunExec {
        exit_code, stdout, new_ref_hash, new_dataset_hash, stdout_hash,
        recomputed_cmd_hash,
    })
}

