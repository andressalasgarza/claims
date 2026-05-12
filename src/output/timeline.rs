//! Timeline rendering (claim chronology). Split out of output.rs so the
//! parent file judges its dispatch/shared-helpers surface separately.
//!
//! `timeline_load` is exposed pub(super) because the stats module also
//! reads the same tag/agent-filtered claim set when assembling snapshots.

use crate::models::Claim;
use crate::store::Store;
use anyhow::Result;

use super::{agent_excluded, confidence_str, deps_of, refutes_of, state_colored, OutputFormat};

pub(super) fn timeline_load(
    store: &Store,
    tag: Option<&str>,
    exclude_agent: &[String],
) -> Result<Vec<Claim>> {
    let mut claims: Vec<Claim> = store
        .all_claims()?
        .into_iter()
        .filter(|c| match tag {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        })
        .filter(|c| !agent_excluded(c, exclude_agent))
        .collect();
    claims.sort_by_key(|c| c.seq);
    Ok(claims)
}

fn timeline_ai(claims: &[Claim]) -> Result<String> {
    let arr: Vec<_> = claims
        .iter()
        .map(|c| {
            serde_json::json!({
                "seq": c.seq,
                "state": c.state.as_str(),
                "confidence": c.confidence().map(|t| t.as_str()),
                "text": c.text,
                "tags": c.tags,
                "depends_on": deps_of(c),
                "refutes": refutes_of(c),
            })
        })
        .collect();
    Ok(serde_json::to_string(&arr)?)
}

fn timeline_row(c: &Claim) -> String {
    let conf = confidence_str(c);
    let edges = c
        .edges
        .iter()
        .map(|e| format!("{} #{}", e.r#type.as_str(), e.to_seq))
        .collect::<Vec<_>>()
        .join(", ");
    let edges_str = if edges.is_empty() {
        String::new()
    } else {
        format!("  ({})", edges)
    };
    format!(
        "#{:<4} [{:<12} · {:<10}] {}{}\n",
        c.seq,
        state_colored(c.state),
        conf,
        c.text,
        edges_str
    )
}

pub fn render_timeline(
    store: &Store,
    fmt: OutputFormat,
    tag: Option<&str>,
    exclude_agent: &[String],
) -> Result<String> {
    let claims = timeline_load(store, tag, exclude_agent)?;
    if fmt == OutputFormat::Ai {
        return timeline_ai(&claims);
    }
    Ok(claims.iter().map(timeline_row).collect())
}
