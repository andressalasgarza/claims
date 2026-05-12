use anyhow::{anyhow, Result};
use chrono::Utc;
use colored::*;

use crate::cli::RefuteArgs;
use crate::commands::util::{commit_evidence, refuse_in_repair_mode};
use crate::models::{Edge, EdgeType, Evidence, EvidenceMethod, State};
use crate::output::{self, OutputFormat};
use crate::store::{cascade_suspect, Store};

fn refute_evidence(by: u64, reason: String) -> Evidence {
    Evidence {
        method: EvidenceMethod::Derived,
        r#ref: format!("claim:#{}", by),
        note: Some(reason),
        p_value: None,
        sample_size: None,
        test_type: None,
        exit_code: None,
        quote: None,
        from_claims: vec![by],
        ref_hash: None,
        cmd: None,
        cmd_hash: None,
        stdout_hash: None,
        target: None,
        dataset: None,
        dataset_hash: None,
        data_source: None,
        metric: None,
        metric_value: None,
        threshold: None,
        estimator: None,
        point_value: None,
        ci_lower: None,
        ci_upper: None,
        confidence_level: None,
        recorded_at: Utc::now(),
    }
}

fn render_cascade_note(store: &mut Store, seq: u64, cascade: bool) -> Result<String> {
    if cascade {
        let affected = cascade_suspect(store, seq)?;
        if affected.is_empty() {
            return Ok(String::new());
        }
        return Ok(format!(
            "\n{} {} dependent claim(s) auto-flagged suspect: {:?}\n",
            "cascade:".yellow().bold(),
            affected.len(),
            affected
        ));
    }
    let dependents = store.transitive_dependents(seq)?;
    if dependents.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "\n{} {} dependent claim(s) NOT flagged. re-run with --cascade to auto-mark suspect: {:?}\n",
        "warning:".yellow(),
        dependents.len(),
        dependents
    ))
}

pub(crate) fn cmd_refute(store: &mut Store, a: RefuteArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("refute")?;
    let seq = store.resolve(&a.id)?;
    if a.by == seq {
        return Err(anyhow!(
            "refute: claim #{} cannot refute itself. self-refutation is incoherent — if the claim is wrong, write a different claim that contradicts it and refute via that.",
            seq
        ));
    }
    let mut claim = store.read_claim(seq)?;
    let replacement = store.read_claim(a.by)?;
    if !matches!(replacement.state, State::Verified) {
        return Err(anyhow!(
            "refute: --by #{} is {} (not verified). a refutation must replace a claim with a verified counter-claim. either verify #{} first or write a different replacement.",
            a.by,
            replacement.state.as_str(),
            a.by
        ));
    }
    claim.state = State::Refuted;
    claim.edges.push(Edge {
        r#type: EdgeType::Refutes,
        to_seq: a.by,
    });
    claim.evidence.push(refute_evidence(a.by, a.reason));
    commit_evidence(&mut claim, store)?;

    let mut out = output::render_claim(&claim, store, fmt)?;
    out.push_str(&render_cascade_note(store, seq, a.cascade)?);
    print!("{}", out);
    Ok(())
}
