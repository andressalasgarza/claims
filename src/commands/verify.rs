//! `clms verify`: attach evidence to a claim and flip it to Verified.
//! the body is decomposed into 5 phase fns so each can be reasoned about
//! independently:
//!   * verify_run_artifact_method  — exec + populate empirical artifact fields
//!   * verify_run_runnable_method  — exec + exit-code mismatch + loopback gate
//!   * verify_check_drifts         — ref drift / rename / dataset drift
//!   * verify_check_dedup          — refuse duplicate evidence rows
//!   * verify_check_min_tier       — refuse below the stamped tier floor

pub(crate) mod artifact;
pub(crate) mod build;

use anyhow::{anyhow, Result};

use crate::cli::VerifyArgs;
use crate::commands::util::{assert_not_refuted_verify, commit_evidence, refuse_in_repair_mode};
use crate::commands::verify::artifact::{
    assert_artifact_cmd_zero, populate_empirical_artifact_fields, preflight_artifact_driven_inputs,
};
use crate::commands::verify::build::build_evidence;
use crate::models::{Claim, Evidence, EvidenceMethod, State};
use crate::output::{self, OutputFormat};
use crate::store::{
    detect_dataset_drift, detect_drift, detect_rename, evidence_user_hash, hash_ref_if_local,
    run_cmd, target_is_local, validate_evidence, Store,
};

/// run the cmd for artifact-driven methods (stat-test/benchmark/estimate),
/// populate the empirical fields from the json artifact at --ref, and
/// validate. `ev` mutates: exit_code, stdout_hash, ref_hash, and the
/// method-specific artifact fields.
fn verify_run_artifact_method(ev: &mut Evidence, store: &Store, seq: u64) -> Result<()> {
    preflight_artifact_driven_inputs(ev)?;
    let cmd = ev
        .cmd
        .as_deref()
        .expect("preflight guarantees --cmd presence for artifact-driven methods");
    eprintln!("executing: {}", cmd);
    let (actual_exit, stdout) = run_cmd(cmd)?;
    assert_artifact_cmd_zero(ev.method, cmd, actual_exit, &stdout, "verify")?;
    ev.exit_code = Some(actual_exit);
    ev.stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
    ev.ref_hash = hash_ref_if_local(&ev.r#ref);
    populate_empirical_artifact_fields(ev)?;
    validate_evidence(ev, store, seq)
}

/// run the cmd for runnable-but-not-artifact methods (prop / integration /
/// replay) and reject any user-supplied --exit-code that disagrees with the
/// actual exit. without this pass, the falsifiability bar for these methods
/// was an honor system since the contradiction only surfaced on `clms
/// rerun`. also enforces the no-loopback rule for integration-test targets.
fn verify_run_runnable_method(
    ev: &mut Evidence, store: &Store, seq: u64, allow_local: bool,
) -> Result<()> {
    validate_evidence(ev, store, seq)?;
    if matches!(ev.method, EvidenceMethod::IntegrationTest) {
        if let Some(t) = ev.target.as_deref() {
            if target_is_local(t) && !allow_local {
                return Err(anyhow!(
                    "integration-test --target '{}' is a loopback / private-network address. integration-test must hit a real external system the author does not control; a service on your own machine cannot falsify that.\n\npass --allow-local if you intentionally want to record a local smoketest as integration evidence (lower-confidence by convention, but accepted), or use --method observed with a log artifact instead.",
                    t
                ));
            }
        }
    }
    if !matches!(
        ev.method,
        EvidenceMethod::PropTest | EvidenceMethod::IntegrationTest | EvidenceMethod::ReplayTest
    ) {
        return Ok(());
    }
    let cmd = ev
        .cmd
        .as_deref()
        .expect("validate_evidence guarantees --cmd presence for these methods");
    let claimed_exit = ev.exit_code;
    eprintln!("executing: {}", cmd);
    let (actual_exit, stdout) = run_cmd(cmd)?;
    if let Some(claimed) = claimed_exit {
        if claimed != actual_exit {
            let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
            return Err(anyhow!(
                "--exit-code mismatch on claim #{}: you claimed {}, --cmd actually exited {}.\n  cmd:    {}\n  stdout: {:?}{}\n\nclms now executes --cmd at verify time and captures the actual exit_code. either fix --cmd, drop the (now-optional) --exit-code flag, or write a different claim if the contradiction is real.",
                seq,
                claimed,
                actual_exit,
                cmd,
                preview,
                if stdout.len() > 200 { " ..." } else { "" },
            ));
        }
    }
    ev.exit_code = Some(actual_exit);
    ev.stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
    Ok(())
}

/// the three drift gates: ref content changed, ref renamed (same hash, new
/// path), dataset bytes changed. all three short-circuit unless
/// --acknowledge-drift is set. each error string is intentionally distinct
/// so the user knows which gate fired.
fn verify_check_drifts(claim: &Claim, ev: &Evidence, ack: bool, seq: u64) -> Result<()> {
    if let Some((prior_hash, prior_at)) = detect_drift(claim, &ev.r#ref, &ev.ref_hash) {
        if !ack {
            return Err(anyhow!(
                "evidence drift detected on '{}':\n  prior hash: {} (recorded {})\n  new hash:   {}\n\n\
the file behind this --ref has changed since prior evidence on this claim. \
if this iteration is intentional, re-run with --acknowledge-drift. if the \
result contradicts the prior conclusion, you should write a new claim and \
refute this one instead.",
                ev.r#ref,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    if let Some((prior_ref, prior_at)) = detect_rename(claim, &ev.r#ref, &ev.ref_hash) {
        if !ack {
            return Err(anyhow!(
                "ref rename detected: --ref '{}' has the same content_hash as prior evidence on claim #{} which used '{}' (recorded {}).\n\n\
the bytes are identical — this is a rename/copy/symlink of the same file under a new path, not new evidence. without this check the dedup (which keys on the ref *string*) would accept this as independent evidence and inflate the count for free. use the original ref, re-run with --acknowledge-drift if the renamed copy is intentional documentation, or write a new claim if it really refers to a distinct file.",
                ev.r#ref, seq, prior_ref, prior_at
            ));
        }
    }
    if let Some((ds, prior_hash, prior_at)) =
        detect_dataset_drift(claim, ev.dataset.as_deref(), &ev.dataset_hash)
    {
        if !ack {
            return Err(anyhow!(
                "dataset drift detected on '{}':\n  prior dataset_hash: {} (recorded {})\n  new dataset_hash:   {}\n\n\
the replay dataset bytes have changed since prior evidence on this claim, even though the dataset path is unchanged. \
an agent that swaps parquet/csv contents can defeat ref-string drift detection. if this iteration is intentional, \
re-run with --acknowledge-drift. if the new bytes contradict the prior conclusion, write a new claim and refute \
this one instead.",
                ds,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.dataset_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    Ok(())
}

/// dedup gate: refuse if the user-supplied evidence fields collide with an
/// existing entry. without this an agent can stack 5x identical verify
/// calls and inflate the apparent evidence count for free.
fn verify_check_dedup(claim: &Claim, ev: &Evidence, seq: u64) -> Result<()> {
    let new_user_hash = evidence_user_hash(ev);
    if claim.evidence.iter().any(|prior| evidence_user_hash(prior) == new_user_hash) {
        return Err(anyhow!(
            "duplicate evidence: claim #{} already has an entry with the same method, ref, and parameters. add new information (a different ref, exit_code, target, dataset, etc.) or write a new claim.",
            seq
        ));
    }
    Ok(())
}

/// min_tier gate: if the claim was stamped with a tier floor at write time
/// (orchestrator intent: "this is a science claim, demand empirical"),
/// refuse the verify if the method's tier is below the floor. uses the
/// METHODS table for tier lookup so it cannot drift from validation logic.
fn verify_check_min_tier(claim: &Claim, ev: &Evidence, seq: u64) -> Result<()> {
    let Some(floor) = claim.min_tier else { return Ok(()); };
    let ev_tier = ev.method.tier();
    if ev_tier.is_at_least(floor) {
        return Ok(());
    }
    use crate::models::METHODS;
    let allowed: Vec<&str> = METHODS
        .iter()
        .filter(|m| m.tier.is_at_least(floor))
        .map(|m| m.name)
        .collect();
    Err(anyhow!(
        "claim #{} requires min_tier={} (got method '{}' at tier {}). allowed methods: {}.\n\nthis claim was stamped with a tier floor at creation time (likely by an orchestrator declaring it a science claim). either supply evidence at or above the floor, or write a different claim if the lower-tier evidence is the actual finding.",
        seq,
        floor.as_str(),
        ev.method.as_str(),
        ev_tier.as_str(),
        allowed.join(" | "),
    ))
}

/// dispatch the method-specific cmd execution. artifact-driven methods
/// also populate empirical fields from the json at --ref; runnable methods
/// just reconcile exit_code + check loopback. extracted so cmd_verify
/// pays one decision for the whole exec phase.
fn verify_execute_for_method(
    ev: &mut Evidence, store: &Store, seq: u64, allow_local: bool,
) -> Result<()> {
    match ev.method {
        EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate => {
            verify_run_artifact_method(ev, store, seq)
        }
        _ => verify_run_runnable_method(ev, store, seq, allow_local),
    }
}

/// run all three semantic gates (drift / dedup / min-tier) as a single
/// caller decision. each gate is independently testable but cmd_verify
/// doesn't care which one fires.
fn verify_run_gates(claim: &Claim, ev: &Evidence, ack_drift: bool, seq: u64) -> Result<()> {
    verify_check_drifts(claim, ev, ack_drift, seq)?;
    verify_check_dedup(claim, ev, seq)?;
    verify_check_min_tier(claim, ev, seq)?;
    Ok(())
}

/// flip state to Verified, append the evidence, commit, render. the order
/// matters: state must flip before write_claim recomputes content_hash.
fn commit_verified_evidence(
    claim: &mut Claim, store: &mut Store, ev: Evidence, fmt: OutputFormat,
) -> Result<()> {
    claim.evidence.push(ev);
    claim.state = State::Verified;
    commit_evidence(claim, store)?;
    print!("{}", output::render_claim(claim, store, fmt)?);
    Ok(())
}

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
