use crate::models::EvidenceMethod;

/// snapshot of the prior runnable evidence fields cmd_rerun needs after
/// reading the claim. flat owned record so cmd_rerun doesn't carry a
/// borrow into `claim` while later mutating it.
pub(crate) struct PriorEvidence {
    pub(crate) cmd: String,
    pub(crate) r#ref: String,
    pub(crate) method: EvidenceMethod,
    pub(crate) target: Option<String>,
    pub(crate) dataset: Option<String>,
    pub(crate) threshold: Option<f64>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) cmd_hash: Option<String>,
}


/// output of the cmd execution + post-run hashing. bundles the values
/// needed downstream so cmd_rerun pays one `?` for the whole chunk
/// instead of one per binding.
pub(crate) struct RerunExec {
    pub(crate) exit_code: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) new_ref_hash: Option<String>,
    pub(crate) new_dataset_hash: Option<String>,
    pub(crate) stdout_hash: Option<String>,
    pub(crate) recomputed_cmd_hash: String,
}

