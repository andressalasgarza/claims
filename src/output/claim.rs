//! Claim rendering across the three output formats (default / human / ai).
//!
//! Split out of output.rs to keep the parent file judged on dispatch and
//! shared helpers only. All format-specific helpers (header_block / edges_
//! block / evidence_block / human_*_str) live here because they are only
//! consumed by render_claim_{default,human}; render_claim_ai builds its
//! json inline.

use crate::models::{Claim, Evidence, EvidenceMethod};
use crate::store::Store;
use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Table};

use super::{confidence_str, dependents_seqs, state_colored};

pub(super) fn render_claim_ai(c: &Claim, store: &Store) -> Result<String> {
    let val = serde_json::json!({
        "seq": c.seq,
        "id": c.id.to_string(),
        "state": c.state.as_str(),
        "confidence": c.confidence().map(|t| t.as_str()),
        "text": c.text,
        "tags": c.tags,
        "depends_on": super::deps_of(c),
        "dependents": dependents_seqs(store, c.seq)?,
        "evidence_count": c.evidence.len(),
        "backfilled": c.backfilled,
        "min_tier": c.min_tier.map(|t| t.as_str()),
    });
    Ok(serde_json::to_string(&val)?)
}

pub(super) fn evidence_extras(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " [{}, p={}, n={}, src={}]",
            e.test_type.map(|t| t.as_str()).unwrap_or("?"),
            e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::PropTest => format!(" [exit={}]", e.exit_code.unwrap_or(-1)),
        EvidenceMethod::IntegrationTest => format!(
            " [exit={}, target={}]",
            e.exit_code.unwrap_or(-1),
            e.target.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::ReplayTest => format!(
            " [exit={}, dataset={}]",
            e.exit_code.unwrap_or(-1),
            e.dataset.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::Benchmark => format!(
            " [{}, value={}, threshold={}, n={}, src={}]",
            e.metric.map(|m| m.as_str()).unwrap_or("?"),
            e.metric_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.threshold.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Estimate => format!(
            " [{}, point={}, CI=[{}, {}], conf={}, n={}, src={}]",
            e.estimator.map(|m| m.as_str()).unwrap_or("?"),
            e.point_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_lower.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_upper.map(|v| v.to_string()).unwrap_or("?".into()),
            e.confidence_level.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Documented => {
            let q = e.quote.as_deref().unwrap_or("");
            let q = if q.len() > 60 { &q[..60] } else { q };
            format!(" [\"{}\"]", q)
        }
        EvidenceMethod::Derived => format!(" [from {:?}]", e.from_claims),
        _ => String::new(),
    }
}

fn header_block(c: &Claim) -> String {
    let backfill_tag = if c.backfilled { " [backfilled]" } else { "" };
    let min_tier_tag = c
        .min_tier
        .map(|t| format!(" [min_tier={}]", t.as_str()))
        .unwrap_or_default();
    let mut s = format!(
        "#{:<4} [{} · {}]  {}{}{}\n",
        c.seq,
        c.state.as_str(),
        confidence_str(c),
        c.text,
        backfill_tag,
        min_tier_tag,
    );
    s.push_str(&format!(
        "id: {}  agent: {}  session: {}  git: {}\n",
        c.id,
        c.agent.as_deref().unwrap_or("-"),
        c.session.as_deref().unwrap_or("-"),
        c.git_sha.as_deref().unwrap_or("-"),
    ));
    if !c.tags.is_empty() {
        s.push_str(&format!("tags: {}\n", c.tags.join(", ")));
    }
    s
}

fn edges_block(c: &Claim) -> String {
    if c.edges.is_empty() {
        return String::new();
    }
    let mut s = String::from("edges:\n");
    for e in &c.edges {
        s.push_str(&format!("  {} #{}\n", e.r#type.as_str(), e.to_seq));
    }
    s
}

fn incoming_block(store: &Store, seq: u64) -> Result<String> {
    let dependents = store.dependents_of(seq)?;
    if dependents.is_empty() {
        return Ok(String::new());
    }
    let mut s = String::from("incoming:\n");
    for (from, t) in dependents {
        s.push_str(&format!("  #{} {} this\n", from, t));
    }
    Ok(s)
}

fn evidence_block(c: &Claim) -> String {
    if c.evidence.is_empty() {
        return String::new();
    }
    let mut s = format!("evidence ({}):\n", c.evidence.len());
    for e in &c.evidence {
        s.push_str(&format!(
            "  [{}] {}{}\n",
            e.method.as_str(),
            e.r#ref,
            evidence_extras(e)
        ));
        if let Some(n) = &e.note {
            s.push_str(&format!("    note: {}\n", n));
        }
    }
    s
}

pub(super) fn render_claim_default(c: &Claim, store: &Store) -> Result<String> {
    let mut s = header_block(c);
    s.push_str(&edges_block(c));
    s.push_str(&incoming_block(store, c.seq)?);
    s.push_str(&evidence_block(c));
    Ok(s)
}

fn human_edges_str(c: &Claim) -> String {
    c.edges
        .iter()
        .map(|e| format!("{} #{}", e.r#type.as_str(), e.to_seq))
        .collect::<Vec<_>>()
        .join("\n")
}

fn human_incoming_str(store: &Store, seq: u64) -> Result<String> {
    let dependents = store.dependents_of(seq)?;
    Ok(dependents
        .iter()
        .map(|(f, ty)| format!("#{} {}", f, ty))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn human_evidence_str(c: &Claim) -> String {
    c.evidence
        .iter()
        .map(|e| format!("[{}] {}", e.method.as_str(), e.r#ref))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render_claim_human(c: &Claim, store: &Store) -> Result<String> {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL);
    t.add_row(vec!["seq", &format!("#{}", c.seq)]);
    t.add_row(vec!["id", &c.id.to_string()]);
    t.add_row(vec!["state", &state_colored(c.state)]);
    t.add_row(vec!["confidence", confidence_str(c)]);
    t.add_row(vec!["text", &c.text]);
    if !c.tags.is_empty() {
        t.add_row(vec!["tags", &c.tags.join(", ")]);
    }
    let edges = human_edges_str(c);
    if !edges.is_empty() {
        t.add_row(vec!["edges", &edges]);
    }
    let incoming = human_incoming_str(store, c.seq)?;
    if !incoming.is_empty() {
        t.add_row(vec!["incoming", &incoming]);
    }
    let evidence = human_evidence_str(c);
    if !evidence.is_empty() {
        t.add_row(vec!["evidence", &evidence]);
    }
    Ok(t.to_string())
}
