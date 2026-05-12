//! `clms rerun`: re-execute a claim's stored cmd and append fresh
//! evidence. tamper-gates the stored cmd_hash so a hand-edited
//! .claims/*.json cannot become arbitrary shell execution.

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::cli::RerunArgs;
use crate::commands::util::{assert_not_refuted_rerun, commit_evidence, refuse_in_repair_mode};
use crate::commands::verify::artifact::{
    assert_artifact_cmd_zero, populate_empirical_artifact_fields,
};
use crate::models::{Claim, Evidence, EvidenceMethod, State};
use crate::output::{self, OutputFormat};
use crate::store::{
    detect_drift, hash_ref_if_local, latest_runnable_evidence, run_cmd, validate_evidence, Store,
};

/// snapshot of the prior runnable evidence fields cmd_rerun needs after
/// reading the claim. flat owned record so cmd_rerun doesn't carry a
/// borrow into `claim` while later mutating it.
struct PriorEvidence {
    cmd: String,
    r#ref: String,
    method: EvidenceMethod,
    target: Option<String>,
    dataset: Option<String>,
    threshold: Option<f64>,
    exit_code: Option<i32>,
    cmd_hash: Option<String>,
}

/// output of the cmd execution + post-run hashing. bundles the values
/// needed downstream so cmd_rerun pays one `?` for the whole chunk
/// instead of one per binding.
struct RerunExec {
    exit_code: i32,
    stdout: Vec<u8>,
    new_ref_hash: Option<String>,
    new_dataset_hash: Option<String>,
    stdout_hash: Option<String>,
    recomputed_cmd_hash: String,
}

/// execute the prior cmd and hash everything: ref bytes, dataset bytes,
/// stdout, and the (already-recomputed) cmd_hash. one Result so the
/// caller sees a single failure point.
fn execute_and_hash(prior: &PriorEvidence, recomputed_cmd_hash: String) -> Result<RerunExec> {
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

/// open the claim, refuse refuted state, snapshot the prior runnable
/// evidence, and tamper-check the cmd_hash. one entry point, three
/// returns: (claim, prior, recomputed_cmd_hash).
fn open_and_verify_prior(
    store: &Store, id: &str,
) -> Result<(u64, Claim, PriorEvidence, String)> {
    let seq = store.resolve(id)?;
    let claim = store.read_claim(seq)?;
    assert_not_refuted_rerun(&claim, seq)?;
    let prior = load_runnable_prior(&claim, seq)?;
    let recomputed_cmd_hash = blake3::hash(prior.cmd.as_bytes()).to_hex().to_string();
    assert_cmd_untampered(&prior, &recomputed_cmd_hash, seq)?;
    Ok((seq, claim, prior, recomputed_cmd_hash))
}

/// post-execution tail: validate (if artifact-driven), push evidence,
/// handle the contradiction-gate (which may itself flip + commit + render
/// + Err), then commit + render the happy path. bundles the 5 ?s after
/// build_rerun_evidence into one caller decision.
fn finalize_rerun(
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

/// locate the most recent runnable evidence on the claim and snapshot
/// the fields cmd_rerun needs. errors with a method-list hint when nothing
/// runnable exists.
fn load_runnable_prior(claim: &Claim, seq: u64) -> Result<PriorEvidence> {
    let prior = latest_runnable_evidence(claim).ok_or_else(|| {
        use crate::models::METHODS;
        let runnable_names: Vec<&str> = METHODS
            .iter()
            .filter(|m| m.runnable)
            .map(|m| m.name)
            .collect();
        anyhow!(
            "claim #{} has no runnable evidence (need one of {} with stored --cmd)",
            seq,
            runnable_names.join(", ")
        )
    })?;
    Ok(PriorEvidence {
        cmd: prior.cmd.clone().unwrap(),
        r#ref: prior.r#ref.clone(),
        method: prior.method,
        target: prior.target.clone(),
        dataset: prior.dataset.clone(),
        threshold: prior.threshold,
        exit_code: prior.exit_code,
        cmd_hash: prior.cmd_hash.clone(),
    })
}

/// tamper-gate: re-hash the stored cmd and refuse to execute if the json
/// was edited since verify time. without this check, anyone who can write
/// to a .claims/*.json gets arbitrary code execution via `clms rerun`.
fn assert_cmd_untampered(prior: &PriorEvidence, recomputed_hash: &str, seq: u64) -> Result<()> {
    match prior.cmd_hash.as_deref() {
        None => Err(anyhow!(
            "claim #{} has runnable evidence with no cmd_hash (pre-tamper-gate format). \
 re-verify with the current binary to record cmd_hash, then rerun.",
            seq
        )),
        Some(prior_hash) if prior_hash != recomputed_hash => Err(anyhow!(
            "refusing to rerun: cmd has been tampered since verify time.\n  expected cmd_hash: {}\n  actual cmd_hash:   {}\n  cmd:               {:?}\n\nthe `cmd` field in the on-disk evidence record was edited after verify. \
rerun would execute attacker-controlled shell. write a new claim instead.",
            &prior_hash[..16.min(prior_hash.len())],
            &recomputed_hash[..16],
            prior.cmd,
        )),
        _ => Ok(()),
    }
}

/// drift-gate: refuse if the local --ref bytes changed since prior evidence.
/// no-op when --acknowledge-drift is set.
fn assert_no_drift(claim: &Claim, prior: &PriorEvidence, new_ref_hash: &Option<String>, ack: bool) -> Result<()> {
    if ack {
        return Ok(());
    }
    let Some((prior_hash, prior_at)) = detect_drift(claim, &prior.r#ref, new_ref_hash) else {
        return Ok(());
    };
    Err(anyhow!(
        "test file '{}' has changed since prior evidence:\n  prior hash: {} (recorded {})\n  new hash:   {}\n\nrun with --acknowledge-drift to record this rerun anyway.",
        prior.r#ref,
        &prior_hash[..16.min(prior_hash.len())],
        prior_at,
        new_ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
    ))
}

/// construct the rerun's Evidence record. all method-specific fields
/// (metric_value, point_value, etc.) are None and get populated downstream
/// by populate_empirical_artifact_fields when the method is artifact-driven.
fn build_rerun_evidence(
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

/// for artifact-driven methods, assert exit==0, populate empirical fields
/// from the json at --ref, then validate. for other methods, no-op.
fn maybe_validate_artifact_evidence(
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
