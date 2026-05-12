use super::snapshot::{pct, tier_count, StatsSnapshot};
use anyhow::Result;

pub(super) fn render_stats_ai(s: &StatsSnapshot) -> Result<String> {
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
