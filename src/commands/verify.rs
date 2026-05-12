//! `clms verify`: attach evidence to a claim and flip it to Verified.
//! orchestration stays here; execution, gates, evidence building, and commit
//! rendering live in focused submodules.

pub(crate) mod artifact;
pub(crate) mod build;

mod commit;
mod execute;
mod gates;

use anyhow::Result;

use crate::cli::VerifyArgs;
use crate::commands::util::{assert_not_refuted_verify, refuse_in_repair_mode};
use crate::commands::verify::build::build_evidence;
use crate::models::EvidenceMethod;
use crate::store::Store;
use crate::output::OutputFormat;

use commit::commit_verified_evidence;
use execute::verify_execute_for_method;
use gates::verify_run_gates;

pub(crate) fn cmd_verify(store: &mut Store, a: VerifyArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("verify")?;
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    assert_not_refuted_verify(&claim, seq)?;
    let m = EvidenceMethod::parse(&a.method)?;
    let mut ev = build_evidence(&a, m)?;
    verify_execute_for_method(&mut ev, store, seq, a.allow_local)?;
    verify_run_gates(&claim, &ev, a.acknowledge_drift, seq)?;
    commit_verified_evidence(&mut claim, store, ev, fmt)
}
