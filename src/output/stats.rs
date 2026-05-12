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

use crate::models::{Claim, ConfidenceTier, State, METHODS};
use crate::store::Store;
use anyhow::Result;

use super::timeline::timeline_load;
use super::OutputFormat;

struct StatsSnapshot {
    total: usize,
    by_state: Vec<(&'static str, usize)>,
    verified_total: usize,
    by_tier: Vec<(&'static str, usize)>, // ordered: empirical, observed, documented, derived
    evidence_total: usize,
    by_method: Vec<(&'static str, usize)>, // ordered as METHODS declares
}

fn state_name(s: State) -> &'static str {
    match s {
        State::Pending => "pending",
        State::Verified => "verified",
        State::Refuted => "refuted",
        State::Suspect => "suspect",
        State::Unverifiable => "unverifiable",
    }
}

fn collect_stats(
    store: &Store,
    tag: Option<&str>,
    exclude_agent: &[String],
) -> Result<StatsSnapshot> {
    let claims = timeline_load(store, tag, exclude_agent)?;
    let total = claims.len();

    // by_state: stable order matching state enum
    let states = [
        State::Verified,
        State::Pending,
        State::Refuted,
        State::Suspect,
        State::Unverifiable,
    ];
    let by_state: Vec<(&'static str, usize)> = states
        .iter()
        .map(|s| (state_name(*s), claims.iter().filter(|c| c.state == *s).count()))
        .collect();

    // by_tier: only verified claims, counted at their top tier.
    let verified: Vec<&Claim> = claims.iter().filter(|c| c.state == State::Verified).collect();
    let verified_total = verified.len();
    let tier_order = [
        ConfidenceTier::Empirical,
        ConfidenceTier::Observed,
        ConfidenceTier::Documented,
        ConfidenceTier::Derived,
    ];
    let by_tier: Vec<(&'static str, usize)> = tier_order
        .iter()
        .map(|t| {
            let count = verified
                .iter()
                .filter(|c| c.confidence() == Some(*t))
                .count();
            (t.as_str(), count)
        })
        .collect();

    // by_method: evidence rows across ALL claims (including refuted etc),
    // counted at method granularity. method order matches METHODS so the
    // output is stable and matches schema dumps.
    let mut method_counts: Vec<(&'static str, usize)> = METHODS
        .iter()
        .map(|spec| (spec.name, 0usize))
        .collect();
    let mut evidence_total = 0usize;
    for c in &claims {
        for ev in &c.evidence {
            evidence_total += 1;
            let name = ev.method.as_str();
            if let Some(slot) = method_counts.iter_mut().find(|(n, _)| *n == name) {
                slot.1 += 1;
            }
        }
    }

    Ok(StatsSnapshot {
        total,
        by_state,
        verified_total,
        by_tier,
        evidence_total,
        by_method: method_counts,
    })
}

/// lookup a tier count from the `by_tier` snapshot. extracted because both
/// render_stats_human and render_stats_ai need the (empirical, observed)
/// pair and the lookup-or-zero pattern was copy-pasted ~7 lines each.
fn tier_count(s: &StatsSnapshot, tier: &str) -> usize {
    s.by_tier
        .iter()
        .find(|(n, _)| *n == tier)
        .map(|(_, c)| *c)
        .unwrap_or(0)
}

fn pct(num: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64
    }
}

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

fn render_stats_human(s: &StatsSnapshot) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!("claims: {} total\n", s.total));

    out.push_str("\nby state:\n");
    for (name, count) in &s.by_state {
        out.push_str(&format!(
            "  {:<14} {:>4}  ({:>4.0}%)\n",
            name,
            count,
            pct(*count, s.total) * 100.0
        ));
    }

    out.push_str(&format!(
        "\nby tier (verified only, {} claims):\n",
        s.verified_total
    ));
    for (name, count) in &s.by_tier {
        out.push_str(&format!(
            "  {:<14} {:>4}  ({:>4.0}%)\n",
            name,
            count,
            pct(*count, s.verified_total) * 100.0
        ));
    }

    out.push_str(&format!(
        "\nby method (evidence rows, {} total):\n",
        s.evidence_total
    ));
    for (name, count) in &s.by_method {
        // suppress zero rows in human output to keep it scannable; ai keeps everything.
        if *count == 0 {
            continue;
        }
        out.push_str(&format!(
            "  {:<18} {:>4}  ({:>4.0}%)\n",
            name,
            count,
            pct(*count, s.evidence_total) * 100.0
        ));
    }

    // observed/empirical ratio: the shopping-down signal.
    let empirical = tier_count(s, "empirical");
    let observed = tier_count(s, "observed");
    out.push_str("\nratios:\n");
    if empirical == 0 {
        out.push_str(&format!(
            "  observed/empirical:    n/a  (empirical=0, observed={})\n",
            observed
        ));
    } else {
        out.push_str(&format!(
            "  observed/empirical:  {:.2}  (observed={}, empirical={})\n",
            observed as f64 / empirical as f64,
            observed,
            empirical
        ));
    }

    Ok(out)
}

fn render_stats_ai(s: &StatsSnapshot) -> Result<String> {
    let by_state: serde_json::Map<String, serde_json::Value> = s
        .by_state
        .iter()
        .map(|(n, c)| {
            (
                (*n).to_string(),
                serde_json::json!({ "count": c, "pct": pct(*c, s.total) }),
            )
        })
        .collect();

    let by_tier: serde_json::Map<String, serde_json::Value> = s
        .by_tier
        .iter()
        .map(|(n, c)| {
            (
                (*n).to_string(),
                serde_json::json!({ "count": c, "pct": pct(*c, s.verified_total) }),
            )
        })
        .collect();

    let by_method: serde_json::Map<String, serde_json::Value> = s
        .by_method
        .iter()
        .map(|(n, c)| {
            (
                (*n).to_string(),
                serde_json::json!({ "count": c, "pct": pct(*c, s.evidence_total) }),
            )
        })
        .collect();

    let empirical = tier_count(s, "empirical");
    let observed = tier_count(s, "observed");
    let ratio = if empirical == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!(observed as f64 / empirical as f64)
    };

    let v = serde_json::json!({
        "total": s.total,
        "verified": s.verified_total,
        "evidence_total": s.evidence_total,
        "by_state": by_state,
        "by_tier": by_tier,
        "by_method": by_method,
        "ratios": { "observed_per_empirical": ratio }
    });
    Ok(serde_json::to_string_pretty(&v)? + "\n")
}
