use super::super::{Candidate, SourceMeta, StakeSignal, CLAIM_MARKERS, EVIDENCE_MARKERS};
use super::candidate_id;
use super::parse::parse_evidence_directive;
use super::walk::walk_dir;
use crate::git;
use anyhow::Result;
use std::fs;
use std::path::Path;

pub(crate) fn harvest_annotations(root: &Path) -> Result<Vec<Candidate>> {
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
