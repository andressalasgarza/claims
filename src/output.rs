use crate::models::{Claim, ConfidenceTier, EdgeType, Evidence, EvidenceMethod, State, METHODS};
use crate::store::Store;
use anyhow::Result;
use colored::*;
use comfy_table::{presets::UTF8_FULL, Table};

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
        OutputFormat::Ai => render_claim_ai(c, store),
        OutputFormat::Human => render_claim_human(c, store),
        OutputFormat::Default => render_claim_default(c, store),
    }
}

fn deps_of(c: &Claim) -> Vec<u64> {
    c.edges
        .iter()
        .filter(|e| e.r#type == EdgeType::DependsOn)
        .map(|e| e.to_seq)
        .collect()
}

fn refutes_of(c: &Claim) -> Vec<u64> {
    c.edges
        .iter()
        .filter(|e| e.r#type == EdgeType::Refutes)
        .map(|e| e.to_seq)
        .collect()
}

fn dependents_seqs(store: &Store, seq: u64) -> Result<Vec<u64>> {
    Ok(store
        .dependents_of(seq)?
        .into_iter()
        .filter(|(_, t)| t == "depends_on")
        .map(|(s, _)| s)
        .collect())
}

fn confidence_str(c: &Claim) -> &'static str {
    c.confidence().map(|t| t.as_str()).unwrap_or("none")
}

fn state_colored(state: State) -> String {
    match state {
        State::Verified => format!("{}", "verified".green().bold()),
        State::Refuted => format!("{}", "refuted".red().bold()),
        State::Suspect => format!("{}", "suspect".yellow().bold()),
        State::Pending => format!("{}", "pending".dimmed()),
        State::Unverifiable => format!("{}", "unverifiable".magenta()),
    }
}

fn render_claim_ai(c: &Claim, store: &Store) -> Result<String> {
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

fn evidence_extras(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " [{}, p={}, n={}, src={}]",
            e.test_type.map(|t| t.as_str()).unwrap_or("?"),
            e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::PropTest => format!(" [exit={}]", e.exit_code.unwrap_or(-1)),
        EvidenceMethod::IntegrationTest => format!(
            " [exit={}, target={}]",
            e.exit_code.unwrap_or(-1),
            e.target.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::ReplayTest => format!(
            " [exit={}, dataset={}]",
            e.exit_code.unwrap_or(-1),
            e.dataset.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::Benchmark => format!(
            " [{}, value={}, threshold={}, n={}, src={}]",
            e.metric.map(|m| m.as_str()).unwrap_or("?"),
            e.metric_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.threshold.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Estimate => format!(
            " [{}, point={}, CI=[{}, {}], conf={}, n={}, src={}]",
            e.estimator.map(|m| m.as_str()).unwrap_or("?"),
            e.point_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_lower.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_upper.map(|v| v.to_string()).unwrap_or("?".into()),
            e.confidence_level.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Documented => {
            let q = e.quote.as_deref().unwrap_or("");
            let q = if q.len() > 60 { &q[..60] } else { q };
            format!(" [\"{}\"]", q)
        }
        EvidenceMethod::Derived => format!(" [from {:?}]", e.from_claims),
        _ => String::new(),
    }
}

fn header_block(c: &Claim) -> String {
    let backfill_tag = if c.backfilled { " [backfilled]" } else { "" };
    let min_tier_tag = c
        .min_tier
        .map(|t| format!(" [min_tier={}]", t.as_str()))
        .unwrap_or_default();
    let mut s = format!(
        "#{:<4} [{} · {}]  {}{}{}\n",
        c.seq,
        c.state.as_str(),
        confidence_str(c),
        c.text,
        backfill_tag,
        min_tier_tag,
    );
    s.push_str(&format!(
        "id: {}  agent: {}  session: {}  git: {}\n",
        c.id,
        c.agent.as_deref().unwrap_or("-"),
        c.session.as_deref().unwrap_or("-"),
        c.git_sha.as_deref().unwrap_or("-"),
    ));
    if !c.tags.is_empty() {
        s.push_str(&format!("tags: {}\n", c.tags.join(", ")));
    }
    s
}

fn edges_block(c: &Claim) -> String {
    if c.edges.is_empty() {
        return String::new();
    }
    let mut s = String::from("edges:\n");
    for e in &c.edges {
        s.push_str(&format!("  {} #{}\n", e.r#type.as_str(), e.to_seq));
    }
    s
}

fn incoming_block(store: &Store, seq: u64) -> Result<String> {
    let dependents = store.dependents_of(seq)?;
    if dependents.is_empty() {
        return Ok(String::new());
    }
    let mut s = String::from("incoming:\n");
    for (from, t) in dependents {
        s.push_str(&format!("  #{} {} this\n", from, t));
    }
    Ok(s)
}

fn evidence_block(c: &Claim) -> String {
    if c.evidence.is_empty() {
        return String::new();
    }
    let mut s = format!("evidence ({}):\n", c.evidence.len());
    for e in &c.evidence {
        s.push_str(&format!(
            "  [{}] {}{}\n",
            e.method.as_str(),
            e.r#ref,
            evidence_extras(e)
        ));
        if let Some(n) = &e.note {
            s.push_str(&format!("    note: {}\n", n));
        }
    }
    s
}

fn render_claim_default(c: &Claim, store: &Store) -> Result<String> {
    let mut s = header_block(c);
    s.push_str(&edges_block(c));
    s.push_str(&incoming_block(store, c.seq)?);
    s.push_str(&evidence_block(c));
    Ok(s)
}

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

fn render_claim_human(c: &Claim, store: &Store) -> Result<String> {
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

fn timeline_load(store: &Store, tag: Option<&str>, exclude_agent: &[String]) -> Result<Vec<Claim>> {
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

// ---------------------------------------------------------------------------
// stats
//
// ledger composition snapshot. exposes counts + percentages + the
// observed/empirical ratio so an external policy validator (xagent) can
// enforce mission-level discipline without clms baking opinions in.
//
// counting model:
//   by_state  -> every claim once (state is total)
//   by_tier   -> every VERIFIED claim once at its top tier (max of
//                attached evidence). unverified claims have no settled
//                tier and are excluded.
//   by_method -> evidence ROWS (not claims). a single claim with 3
//                evidence rows contributes 3.
//
// percentages are computed against the relevant total (verified-claim
// count for by_tier, evidence-row count for by_method). ratios use the
// by_tier counts.
// ---------------------------------------------------------------------------

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
