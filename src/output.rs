use crate::models::{Claim, EdgeType, Evidence, EvidenceMethod, State};
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
    });
    Ok(serde_json::to_string(&val)?)
}

fn evidence_extras(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " [{}, p={}, n={}, src={}]",
            e.test_type.as_deref().unwrap_or("?"),
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
    let mut s = format!(
        "#{:<4} [{} · {}]  {}\n",
        c.seq,
        c.state.as_str(),
        confidence_str(c),
        c.text
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
    let seqs = store.all_seqs()?;
    let mut claims: Vec<Claim> = seqs
        .iter()
        .map(|s| store.read_claim(*s))
        .collect::<Result<Vec<_>>>()?
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
    let seqs = store.all_seqs()?;
    let claims: Vec<Claim> = seqs
        .iter()
        .map(|s| store.read_claim(*s))
        .collect::<Result<Vec<_>>>()?
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
