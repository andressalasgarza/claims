use super::super::{dependents_seqs, deps_of};
use crate::models::Claim;
use crate::store::Store;
use anyhow::Result;

pub(crate) fn render_claim_ai(c: &Claim, store: &Store) -> Result<String> {
    let val = serde_json::json!({
        "seq": c.seq,
        "id": c.id.to_string(),
        "state": c.state.as_str(),
        "confidence": c.confidence().map(|t| t.as_str()),
        "text": c.text,
        "tags": c.tags,
        "depends_on": deps_of(c),
        "dependents": dependents_seqs(store, c.seq)?,
        "evidence_count": c.evidence.len(),
        "backfilled": c.backfilled,
        "min_tier": c.min_tier.map(|t| t.as_str()),
    });
    Ok(serde_json::to_string(&val)?)
}
