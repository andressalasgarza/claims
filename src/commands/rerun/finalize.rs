use super::artifact::maybe_validate_artifact_evidence;
use super::types::RerunExec;
use crate::commands::util::commit_evidence;
use crate::models::{Claim, Evidence, State};
use crate::output::{self, OutputFormat};
use crate::store::Store;
use anyhow::{anyhow, Result};

/// post-execution tail: validate (if artifact-driven), push evidence,
/// handle the contradiction-gate (which may itself flip + commit + render
/// + Err), then commit + render the happy path. bundles the 5 ?s after
/// build_rerun_evidence into one caller decision.
pub(super) fn finalize_rerun(
    claim: &mut Claim, store: &mut Store, fmt: OutputFormat, ev: Evidence,
    exec: &RerunExec, prior_exit: Option<i32>, seq: u64,
) -> Result<()> {
    let mut ev = ev;
    maybe_validate_artifact_evidence(&mut ev, store, exec.exit_code, &exec.stdout, seq)?;
    claim.evidence.push(ev);
    handle_rerun_contradiction(claim, store, fmt, prior_exit, exec.exit_code, seq)?;
    commit_evidence(claim, store)?;
    print!("{}", output::render_claim(claim, store, fmt)?);
    Ok(())
}

/// contradiction-gate: if the rerun exit_code disagrees with what was
/// recorded at verify time, the claim's verified status is now suspect.
/// record the new evidence (audit trail), flip state, write, then exit 1.
/// without this, an agent can lie `--exit-code 0` on a script that exits
/// 1, and `rerun` faithfully captures exit=1 but the claim stays verified.
fn handle_rerun_contradiction(
    claim: &mut Claim, store: &mut Store, fmt: OutputFormat,
    prior_exit: Option<i32>, exit_code: i32, seq: u64,
) -> Result<()> {
    let (prev, now) = match (prior_exit, exit_code) {
        (Some(prev), now) if prev != now => (prev, now),
        _ => return Ok(()),
    };
    if matches!(claim.state, State::Verified | State::Pending) {
        claim.state = State::Suspect;
    }
    commit_evidence(claim, store)?;
    print!("{}", output::render_claim(claim, store, fmt)?);
    Err(anyhow!(
        "rerun contradicts prior evidence on claim #{}: stored exit_code={}, fresh exit_code={}.\nclaim flipped to suspect. write a new claim and refute this one if the contradiction is real, or investigate the rerun environment if it is spurious.",
        seq, prev, now,
    ))
}

