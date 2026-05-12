use super::snapshot::StatsSnapshot;
use super::super::timeline::timeline_load;
use crate::models::{Claim, ConfidenceTier, State, METHODS};
use crate::store::Store;
use anyhow::Result;

fn state_name(s: State) -> &'static str {
    match s {
        State::Pending => "pending",
        State::Verified => "verified",
        State::Refuted => "refuted",
        State::Suspect => "suspect",
        State::Unverifiable => "unverifiable",
    }
}

pub(super) fn collect_stats(
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

