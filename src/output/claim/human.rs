use super::super::{confidence_str, state_colored};
use crate::models::Claim;
use crate::store::Store;
use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Table};

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

pub(crate) fn render_claim_human(c: &Claim, store: &Store) -> Result<String> {
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
