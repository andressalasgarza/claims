//! Ledger composition stats (counts + percentages + observed/empirical
//! ratio). Split out of output.rs so the parent file judges dispatch and
//! shared helpers separately.
//!
//! Counting model:
//!   by_state  -> every claim once (state is total).
//!   by_tier   -> every VERIFIED claim once at its top tier (max of
//!                attached evidence). unverified claims have no settled
//!                tier and are excluded.
//!   by_method -> evidence ROWS (not claims). a single claim with 3
//!                evidence rows contributes 3.
//!
//! Percentages are computed against the relevant total (verified-claim
//! count for by_tier, evidence-row count for by_method). Ratios use the
//! by_tier counts.

mod ai;
mod collect;
mod human;
mod snapshot;

use crate::store::Store;
use anyhow::Result;

use super::OutputFormat;
use ai::render_stats_ai;
use collect::collect_stats;
use human::render_stats_human;

pub fn render_stats(
    store: &Store,
    fmt: OutputFormat,
    tag: Option<&str>,
    exclude_agent: &[String],
) -> Result<String> {
    let s = collect_stats(store, tag, exclude_agent)?;
    if fmt == OutputFormat::Ai {
        return render_stats_ai(&s);
    }
    render_stats_human(&s)
}

