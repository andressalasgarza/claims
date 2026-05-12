use super::harvest::candidate_id;
use super::{Candidate, SourceMeta, StakeSignal, SuggestedEvidence, SCHEMA_VERSION};
use crate::git;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::Path;

pub(super) fn harvest_proposals(root: &Path) -> Result<Vec<Candidate>> {
    let path = root.join(".archaeology/proposals.json");
    let Some(arr) = load_proposals_manifest(&path)? else {
        return Ok(Vec::new());
    };
    let manifest_mtime = git::file_mtime(&path);
    let mut out = Vec::new();
    for (idx, row) in arr.iter().enumerate() {
        if let Some(c) = parse_proposal_row(&path, idx, row, manifest_mtime)? {
            out.push(c);
        }
    }
    Ok(out)
}

fn load_proposals_manifest(path: &Path) -> Result<Option<Vec<serde_json::Value>>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
    let version = v["version"].as_str().unwrap_or("");
    if version != SCHEMA_VERSION {
        bail!(
            "{}: expected version '{}', got '{}'",
            path.display(), SCHEMA_VERSION, version
        );
    }
    let arr = v["proposals"]
        .as_array()
        .ok_or_else(|| anyhow!("{}: 'proposals' must be an array", path.display()))?
        .clone();
    Ok(Some(arr))
}

fn parse_proposal_row(
    path: &Path,
    idx: usize,
    row: &serde_json::Value,
    manifest_mtime: Option<DateTime<Utc>>,
) -> Result<Option<Candidate>> {
    let text = row["text"]
        .as_str()
        .ok_or_else(|| anyhow!("{}: proposals[{}].text missing", path.display(), idx))?
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let where_str = row["where"]
        .as_str()
        .ok_or_else(|| anyhow!("{}: proposals[{}].where missing", path.display(), idx))?
        .to_string();
    let snippet = row["snippet"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("// (proposal) clms-claim: {}", text));
    let suggested_evidence = parse_suggested_evidence(row.get("suggested_evidence"));
    let id = candidate_id("clms-claim-annotation", &text, &where_str);
    Ok(Some(Candidate {
        id,
        kind: "clms-claim-annotation".into(),
        text,
        stake_signal: StakeSignal { r#where: where_str, snippet },
        suggested_evidence,
        source_meta: SourceMeta {
            file: ".archaeology/proposals.json".to_string(),
            line: (idx as u32) + 1,
        },
        created_at: manifest_mtime,
        debate: None,
    }))
}

fn parse_suggested_evidence(arr: Option<&serde_json::Value>) -> Vec<SuggestedEvidence> {
    let Some(items) = arr.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|ev| {
            let method = ev["method"].as_str()?.trim();
            if method.is_empty() {
                return None;
            }
            Some(SuggestedEvidence {
                method: method.to_string(),
                cmd: ev["cmd"].as_str().map(String::from),
                r#ref: ev["ref"].as_str().map(String::from),
                note: ev["note"]
                    .as_str()
                    .unwrap_or("advisory only; not run by archaeology. promotion via `clms verify`")
                    .to_string(),
            })
        })
        .collect()
}
