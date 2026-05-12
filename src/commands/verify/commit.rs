use crate::commands::util::commit_evidence;
use crate::models::{Claim, Evidence, State};
use crate::output::{self, OutputFormat};
use crate::store::Store;
use anyhow::Result;

/// flip state to Verified, append the evidence, commit, render. the order
/// matters: state must flip before write_claim recomputes content_hash.
pub(super) fn commit_verified_evidence(
    claim: &mut Claim, store: &mut Store, ev: Evidence, fmt: OutputFormat,
) -> Result<()> {
    claim.evidence.push(ev);
    claim.state = State::Verified;
    commit_evidence(claim, store)?;
    print!("{}", output::render_claim(claim, store, fmt)?);
    Ok(())
}

