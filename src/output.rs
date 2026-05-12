//! Output dispatch + shared helpers.
//!
//! Three formats are exposed: Default, Human (tables via comfy_table), Ai
//! (json). All renderers route through one of three public entry points:
//! `render_claim` (single-claim detail), `render_timeline` (chronology),
//! `render_stats` (ledger composition snapshot). `render_context` stays
//! here because it is a single-function surface with no helpers worth
//! splitting.
//!
//! The shared helpers (confidence_str, state_colored, deps_of, refutes_of,
//! dependents_seqs) sit at module root as pub(super) so each sub-module
//! (claim / timeline / stats) imports them via `use super::*`. agent_excluded
//! is pub(crate) because commands::suspect filters on it directly.

mod claim;
mod stats;
mod timeline;

pub use stats::render_stats;
pub use timeline::render_timeline;

use crate::models::{Claim, EdgeType, State};
use crate::store::Store;
use anyhow::Result;
use colored::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Default,
    Human,
    Ai,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "human" => Ok(Self::Human),
            "ai" => Ok(Self::Ai),
            other => Err(anyhow::anyhow!(
                "invalid format '{}'. valid: default|human|ai",
                other
            )),
        }
    }
}

pub fn render_claim(c: &Claim, store: &Store, fmt: OutputFormat) -> Result<String> {
    match fmt {
        OutputFormat::Ai => claim::render_claim_ai(c, store),
        OutputFormat::Human => claim::render_claim_human(c, store),
        OutputFormat::Default => claim::render_claim_default(c, store),
    }
}

// ---------------------------------------------------------------------------
// shared helpers (visible to sub-modules via pub(super))
// ---------------------------------------------------------------------------

pub(super) fn deps_of(c: &Claim) -> Vec<u64> {
    c.edges
        .iter()
        .filter(|e| e.r#type == EdgeType::DependsOn)
        .map(|e| e.to_seq)
        .collect()
}

pub(super) fn refutes_of(c: &Claim) -> Vec<u64> {
    c.edges
        .iter()
        .filter(|e| e.r#type == EdgeType::Refutes)
        .map(|e| e.to_seq)
        .collect()
}

pub(super) fn dependents_seqs(store: &Store, seq: u64) -> Result<Vec<u64>> {
    Ok(store
        .dependents_of(seq)?
        .into_iter()
        .filter(|(_, t)| t == "depends_on")
        .map(|(s, _)| s)
        .collect())
}

pub(super) fn confidence_str(c: &Claim) -> &'static str {
    c.confidence().map(|t| t.as_str()).unwrap_or("none")
}

pub(super) fn state_colored(state: State) -> String {
    match state {
        State::Verified => format!("{}", "verified".green().bold()),
        State::Refuted => format!("{}", "refuted".red().bold()),
        State::Suspect => format!("{}", "suspect".yellow().bold()),
        State::Pending => format!("{}", "pending".dimmed()),
        State::Unverifiable => format!("{}", "unverifiable".magenta()),
    }
}

/// shared filter: returns true if claim's agent is in the exclude list.
/// used by timeline + suspect rendering and by main's cmd_suspect.
pub(crate) fn agent_excluded(c: &Claim, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    match &c.agent {
        Some(a) => exclude.iter().any(|e| e == a),
        None => false,
    }
}

pub fn render_context(
    store: &Store,
    tag: Option<&str>,
    fmt: OutputFormat,
    exclude_agent: &[String],
) -> Result<String> {
    let claims: Vec<Claim> = store
        .all_claims()?
        .into_iter()
        .filter(|c| matches!(c.state, State::Verified))
        .filter(|c| match tag {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        })
        .filter(|c| !agent_excluded(c, exclude_agent))
        .collect();

    if fmt == OutputFormat::Ai {
        let arr: Vec<_> = claims
            .iter()
            .map(|c| {
                serde_json::json!({
                    "seq": c.seq,
                    "confidence": c.confidence().map(|t| t.as_str()),
                    "text": c.text,
                    "tags": c.tags,
                })
            })
            .collect();
        return Ok(serde_json::to_string(&arr)?);
    }

    let mut out = format!("verified claims context ({} entries)\n", claims.len());
    for c in &claims {
        out.push_str(&format!("  #{} [{}] {}\n", c.seq, confidence_str(c), c.text));
    }
    Ok(out)
}
