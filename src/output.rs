use crate::models::Claim;
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

fn render_claim_ai(c: &Claim, store: &Store) -> Result<String> {
    let deps: Vec<u64> = c
        .edges
        .iter()
        .filter(|e| e.r#type == crate::models::EdgeType::DependsOn)
        .map(|e| e.to_seq)
        .collect();
    let dependents: Vec<u64> = store
        .dependents_of(c.seq)?
        .into_iter()
        .filter(|(_, t)| t == "depends_on")
        .map(|(s, _)| s)
        .collect();
    let val = serde_json::json!({
        "seq": c.seq,
        "id": c.id.to_string(),
        "state": c.state.as_str(),
        "confidence": c.confidence().map(|t| t.as_str()),
        "text": c.text,
        "tags": c.tags,
        "depends_on": deps,
        "dependents": dependents,
        "evidence_count": c.evidence.len(),
    });
    Ok(serde_json::to_string(&val)?)
}

fn render_claim_default(c: &Claim, store: &Store) -> Result<String> {
    let mut s = String::new();
    let conf = c
        .confidence()
        .map(|t| t.as_str())
        .unwrap_or("none");
    s.push_str(&format!(
        "#{:<4} [{} · {}]  {}\n",
        c.seq,
        c.state.as_str(),
        conf,
        c.text
    ));
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
    if !c.edges.is_empty() {
        s.push_str("edges:\n");
        for e in &c.edges {
            s.push_str(&format!("  {} #{}\n", e.r#type.as_str(), e.to_seq));
        }
    }
    let dependents = store.dependents_of(c.seq)?;
    if !dependents.is_empty() {
        s.push_str("incoming:\n");
        for (from, t) in dependents {
            s.push_str(&format!("  #{} {} this\n", from, t));
        }
    }
    if !c.evidence.is_empty() {
        s.push_str(&format!("evidence ({}):\n", c.evidence.len()));
        for e in &c.evidence {
            let extras = match e.method {
                crate::models::EvidenceMethod::StatTest => format!(
                    " [{}, p={}, n={}]",
                    e.test_type.as_deref().unwrap_or("?"),
                    e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
                    e.sample_size.map(|v| v.to_string()).unwrap_or("?".into())
                ),
                crate::models::EvidenceMethod::CodeTest => {
                    format!(" [exit={}]", e.exit_code.unwrap_or(-1))
                }
                crate::models::EvidenceMethod::Documented => {
                    let q = e.quote.as_deref().unwrap_or("");
                    let q = if q.len() > 60 { &q[..60] } else { q };
                    format!(" [\"{}\"]", q)
                }
                crate::models::EvidenceMethod::Derived => format!(" [from {:?}]", e.from_claims),
                _ => String::new(),
            };
            s.push_str(&format!(
                "  [{}] {}{}\n",
                e.method.as_str(),
                e.r#ref,
                extras
            ));
            if let Some(n) = &e.note {
                s.push_str(&format!("    note: {}\n", n));
            }
        }
    }
    Ok(s)
}

fn render_claim_human(c: &Claim, store: &Store) -> Result<String> {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL);
    let conf = c.confidence().map(|t| t.as_str()).unwrap_or("none");
    let badge = match c.state {
        crate::models::State::Verified => format!("{}", "verified".green().bold()),
        crate::models::State::Refuted => format!("{}", "refuted".red().bold()),
        crate::models::State::Suspect => format!("{}", "suspect".yellow().bold()),
        crate::models::State::Pending => format!("{}", "pending".dimmed()),
        crate::models::State::Unverifiable => format!("{}", "unverifiable".magenta()),
    };
    t.add_row(vec!["seq", &format!("#{}", c.seq)]);
    t.add_row(vec!["id", &c.id.to_string()]);
    t.add_row(vec!["state", &badge]);
    t.add_row(vec!["confidence", conf]);
    t.add_row(vec!["text", &c.text]);
    if !c.tags.is_empty() {
        t.add_row(vec!["tags", &c.tags.join(", ")]);
    }
    let edge_str = c
        .edges
        .iter()
        .map(|e| format!("{} #{}", e.r#type.as_str(), e.to_seq))
        .collect::<Vec<_>>()
        .join("\n");
    if !edge_str.is_empty() {
        t.add_row(vec!["edges", &edge_str]);
    }
    let dependents = store.dependents_of(c.seq)?;
    if !dependents.is_empty() {
        let s = dependents
            .iter()
            .map(|(f, ty)| format!("#{} {}", f, ty))
            .collect::<Vec<_>>()
            .join("\n");
        t.add_row(vec!["incoming", &s]);
    }
    if !c.evidence.is_empty() {
        let s = c
            .evidence
            .iter()
            .map(|e| format!("[{}] {}", e.method.as_str(), e.r#ref))
            .collect::<Vec<_>>()
            .join("\n");
        t.add_row(vec!["evidence", &s]);
    }
    Ok(t.to_string())
}

pub fn render_timeline(store: &Store, fmt: OutputFormat, tag: Option<&str>) -> Result<String> {
    let seqs = store.all_seqs()?;
    let mut claims: Vec<Claim> = seqs
        .iter()
        .filter_map(|s| store.read_claim(*s).ok())
        .filter(|c| match tag {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        })
        .collect();
    claims.sort_by_key(|c| c.seq);

    if fmt == OutputFormat::Ai {
        let arr: Vec<_> = claims
            .iter()
            .map(|c| {
                serde_json::json!({
                    "seq": c.seq,
                    "state": c.state.as_str(),
                    "confidence": c.confidence().map(|t| t.as_str()),
                    "text": c.text,
                    "tags": c.tags,
                    "depends_on": c.edges.iter()
                        .filter(|e| e.r#type == crate::models::EdgeType::DependsOn)
                        .map(|e| e.to_seq).collect::<Vec<_>>(),
                    "refutes": c.edges.iter()
                        .filter(|e| e.r#type == crate::models::EdgeType::Refutes)
                        .map(|e| e.to_seq).collect::<Vec<_>>(),
                })
            })
            .collect();
        return Ok(serde_json::to_string(&arr)?);
    }

    let mut out = String::new();
    for c in &claims {
        let conf = c.confidence().map(|t| t.as_str()).unwrap_or("-");
        let state_str = match c.state {
            crate::models::State::Verified => format!("{}", "verified".green()),
            crate::models::State::Refuted => format!("{}", "refuted".red().dimmed()),
            crate::models::State::Suspect => format!("{}", "suspect".yellow()),
            crate::models::State::Pending => format!("{}", "pending".dimmed()),
            crate::models::State::Unverifiable => format!("{}", "unverifiable".magenta()),
        };
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
        out.push_str(&format!(
            "#{:<4} [{:<12} · {:<10}] {}{}\n",
            c.seq, state_str, conf, c.text, edges_str
        ));
    }
    Ok(out)
}

pub fn render_context(store: &Store, tag: Option<&str>, fmt: OutputFormat) -> Result<String> {
    let seqs = store.all_seqs()?;
    let claims: Vec<Claim> = seqs
        .iter()
        .filter_map(|s| store.read_claim(*s).ok())
        .filter(|c| matches!(c.state, crate::models::State::Verified))
        .filter(|c| match tag {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        })
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

    let mut out = String::new();
    out.push_str(&format!(
        "verified claims context ({} entries)\n",
        claims.len()
    ));
    for c in &claims {
        let conf = c.confidence().map(|t| t.as_str()).unwrap_or("-");
        out.push_str(&format!("  #{} [{}] {}\n", c.seq, conf, c.text));
    }
    Ok(out)
}
