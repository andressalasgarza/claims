use super::proposals::harvest_proposals;
use super::{
    Candidate, SourceMeta, StakeSignal, SuggestedEvidence, CLAIM_MARKERS,
    EVIDENCE_MARKERS, IGNORE_DIRS, SOURCE_EXTS,
};
use crate::git;
use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn harvest_all(root: &Path) -> Result<(Vec<Candidate>, usize)> {
    let mut all = harvest_annotations(root)?;
    let proposals = harvest_proposals(root)?;
    let proposals_added = proposals.len();
    all.extend(proposals);

    let mut seen: HashSet<String> = HashSet::new();
    all.retain(|c| seen.insert(c.id.clone()));
    all.sort_by(candidate_chronological_order);

    Ok((all, proposals_added))
}

fn candidate_chronological_order(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    let tiebreak = (&a.source_meta.file, a.source_meta.line)
        .cmp(&(&b.source_meta.file, b.source_meta.line));
    match (&a.created_at, &b.created_at) {
        (Some(x), Some(y)) => x.cmp(y).then(tiebreak),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => tiebreak,
    }
}

fn harvest_annotations(root: &Path) -> Result<Vec<Candidate>> {
    let mut files = Vec::new();
    walk_dir(root, &mut files)?;
    files.sort();

    let mut candidates = Vec::new();
    for f in &files {
        let raw = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        scan_file(f, root, &raw, &mut candidates);
    }
    Ok(candidates)
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if IGNORE_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            walk_dir(&path, out)?;
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SOURCE_EXTS.contains(&ext) {
                    out.push(path);
                }
            }
        }
    }
    Ok(())
}

fn scan_file(path: &Path, root: &Path, content: &str, out: &mut Vec<Candidate>) {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let lines: Vec<&str> = content.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if let Some((marker, claim_text)) = match_marker(line, CLAIM_MARKERS) {
            let claim_text = claim_text.trim().to_string();
            if claim_text.is_empty() {
                continue;
            }
            let snippet = line.trim().to_string();
            let where_str = format!("{}:{}", rel, idx + 1);

            let mut suggested_evidence = Vec::new();
            if let Some(next) = lines.get(idx + 1) {
                if let Some((_, ev_text)) = match_marker(next, EVIDENCE_MARKERS) {
                    suggested_evidence.push(parse_evidence_directive(ev_text));
                }
            }

            let id = candidate_id("clms-claim-annotation", &claim_text, &where_str);
            let created_at = git::blame_line_time(path, (idx as u32) + 1);
            out.push(Candidate {
                id,
                kind: "clms-claim-annotation".into(),
                text: claim_text,
                stake_signal: StakeSignal {
                    r#where: where_str,
                    snippet,
                },
                suggested_evidence,
                source_meta: SourceMeta {
                    file: rel.clone(),
                    line: (idx as u32) + 1,
                },
                created_at,
                debate: None,
            });
            let _ = marker;
        }
    }
}

fn match_marker<'a>(line: &'a str, markers: &[&str]) -> Option<(&'static str, &'a str)> {
    let trimmed = line.trim_start();
    for m in markers {
        if let Some(rest) = trimmed.strip_prefix(m) {
            for static_m in CLAIM_MARKERS.iter().chain(EVIDENCE_MARKERS.iter()) {
                if static_m == m {
                    return Some((static_m, rest));
                }
            }
        }
    }
    None
}

fn parse_evidence_directive(raw: &str) -> SuggestedEvidence {
    let mut method = "prop-test".to_string();
    let mut cmd: Option<String> = None;
    let mut r#ref: Option<String> = None;
    let trimmed = raw.trim();
    for token in split_kv(trimmed) {
        if let Some((k, v)) = token.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "method" => method = v,
                "cmd" => cmd = Some(v),
                "ref" => r#ref = Some(v),
                _ => {}
            }
        }
    }
    SuggestedEvidence {
        method,
        cmd,
        r#ref,
        note: "advisory only; not run by archaeology. promotion via `clms verify`".into(),
    }
}

fn split_kv(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        if c == '"' {
            in_q = !in_q;
            cur.push(c);
        } else if c.is_whitespace() && !in_q {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub(super) fn candidate_id(kind: &str, text: &str, where_str: &str) -> String {
    let payload = format!("{}|{}|{}", kind, text, where_str);
    let h = blake3::hash(payload.as_bytes()).to_hex();
    format!("c-{}", &h.to_string()[..4])
}
