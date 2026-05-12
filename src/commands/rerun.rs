//! `clms rerun`: re-execute a claim's stored cmd and append fresh
//! evidence. tamper-gates the stored cmd_hash so a hand-edited
//! .claims/*.json cannot become arbitrary shell execution.

mod artifact;
mod drift;
mod evidence;
mod exec;
mod finalize;
mod prior;
mod types;

use anyhow::Result;

use crate::cli::RerunArgs;
use crate::commands::util::refuse_in_repair_mode;
use crate::output::OutputFormat;
use crate::store::Store;

use drift::assert_no_drift;
use evidence::build_rerun_evidence;
use exec::execute_and_hash;
use finalize::finalize_rerun;
use prior::open_and_verify_prior;

pub(crate) fn cmd_rerun(store: &mut Store, a: RerunArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("rerun")?;
    let (seq, mut claim, prior, recomputed_cmd_hash) = open_and_verify_prior(store, &a.id)?;
    let exec = execute_and_hash(&prior, recomputed_cmd_hash)?;
    assert_no_drift(&claim, &prior, &exec.new_ref_hash, a.acknowledge_drift)?;
    let ev = build_rerun_evidence(
        &prior, exec.exit_code, exec.new_ref_hash.clone(),
        exec.new_dataset_hash.clone(), exec.stdout_hash.clone(),
        exec.recomputed_cmd_hash.clone(), seq,
    );
    finalize_rerun(&mut claim, store, fmt, ev, &exec, prior.exit_code, seq)
}
