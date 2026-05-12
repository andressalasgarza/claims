use super::snapshot::{pct, tier_count, StatsSnapshot};
use anyhow::Result;

pub(super) fn render_stats_human(s: &StatsSnapshot) -> Result<String> {
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

