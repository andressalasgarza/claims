use crate::commands::verify::artifact::{
    assert_artifact_cmd_zero, populate_empirical_artifact_fields, preflight_artifact_driven_inputs,
};
use crate::models::{Evidence, EvidenceMethod};
use crate::store::{
    hash_ref_if_local, run_cmd, target_is_local, validate_evidence, Store,
};
use anyhow::{anyhow, Result};

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
/// loopback gate: integration-test must hit a real external system the
/// author does not control. a service on the same machine cannot falsify
/// the claim. --allow-local exempts intentional local smoketests.
fn assert_no_loopback_target(method: EvidenceMethod, target: Option<&str>, allow_local: bool) -> Result<()> {
    if !matches!(method, EvidenceMethod::IntegrationTest) || allow_local {
        return Ok(());
    }
    let Some(t) = target else { return Ok(()); };
    if !target_is_local(t) {
        return Ok(());
    }
    Err(anyhow!(
        "integration-test --target '{}' is a loopback / private-network address. integration-test must hit a real external system the author does not control; a service on your own machine cannot falsify that.\n\npass --allow-local if you intentionally want to record a local smoketest as integration evidence (lower-confidence by convention, but accepted), or use --method observed with a log artifact instead.",
        t
    ))
}

/// reject any user-supplied --exit-code that disagrees with the actual cmd
/// exit. without this the falsifiability bar is honor-system since the
/// contradiction would only surface on rerun.
fn assert_exit_code_match(claimed: Option<i32>, actual: i32, cmd: &str, stdout: &[u8], seq: u64) -> Result<()> {
    let Some(c) = claimed else { return Ok(()); };
    if c == actual {
        return Ok(());
    }
    let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
    Err(anyhow!(
        "--exit-code mismatch on claim #{}: you claimed {}, --cmd actually exited {}.\n  cmd:    {}\n  stdout: {:?}{}\n\nclms now executes --cmd at verify time and captures the actual exit_code. either fix --cmd, drop the (now-optional) --exit-code flag, or write a different claim if the contradiction is real.",
        seq, c, actual, cmd, preview,
        if stdout.len() > 200 { " ..." } else { "" },
    ))
}

fn verify_run_runnable_method(
    ev: &mut Evidence, store: &Store, seq: u64, allow_local: bool,
) -> Result<()> {
    validate_evidence(ev, store, seq)?;
    assert_no_loopback_target(ev.method, ev.target.as_deref(), allow_local)?;
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
    eprintln!("executing: {}", cmd);
    let (actual_exit, stdout) = run_cmd(cmd)?;
    assert_exit_code_match(ev.exit_code, actual_exit, cmd, &stdout, seq)?;
    ev.exit_code = Some(actual_exit);
    ev.stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
    Ok(())
}

/// dispatch the method-specific cmd execution. artifact-driven methods
/// also populate empirical fields from the json at --ref; runnable methods
/// just reconcile exit_code + check loopback. extracted so cmd_verify
/// pays one decision for the whole exec phase.
pub(super) fn verify_execute_for_method(
    ev: &mut Evidence, store: &Store, seq: u64, allow_local: bool,
) -> Result<()> {
    match ev.method {
        EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate => {
            verify_run_artifact_method(ev, store, seq)
        }
        _ => verify_run_runnable_method(ev, store, seq, allow_local),
    }
}

