use super::super::confidence_str;
use super::evidence::evidence_extras;
use crate::models::Claim;
use crate::store::Store;
use anyhow::Result;

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

pub(crate) fn render_claim_default(c: &Claim, store: &Store) -> Result<String> {
    let mut s = header_block(c);
    s.push_str(&edges_block(c));
    s.push_str(&incoming_block(store, c.seq)?);
    s.push_str(&evidence_block(c));
    Ok(s)
}
