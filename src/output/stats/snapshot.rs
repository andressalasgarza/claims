pub(super) struct StatsSnapshot {
    pub(super) total: usize,
    pub(super) by_state: Vec<(&'static str, usize)>,
    pub(super) verified_total: usize,
    pub(super) by_tier: Vec<(&'static str, usize)>, // ordered: empirical, observed, documented, derived
    pub(super) evidence_total: usize,
    pub(super) by_method: Vec<(&'static str, usize)>, // ordered as METHODS declares
}


/// lookup a tier count from the `by_tier` snapshot. extracted because both
/// render_stats_human and render_stats_ai need the (empirical, observed)
/// pair and the lookup-or-zero pattern was copy-pasted ~7 lines each.
pub(super) fn tier_count(s: &StatsSnapshot, tier: &str) -> usize {
    s.by_tier
        .iter()
        .find(|(n, _)| *n == tier)
        .map(|(_, c)| *c)
        .unwrap_or(0)
}

pub(super) fn pct(num: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64
    }
}

