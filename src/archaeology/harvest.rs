use super::proposals::harvest_proposals;
use super::Candidate;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;

mod parse;
mod scan;
mod walk;

use scan::harvest_annotations;

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
    let tiebreak =
        (&a.source_meta.file, a.source_meta.line).cmp(&(&b.source_meta.file, b.source_meta.line));
    match (&a.created_at, &b.created_at) {
        (Some(x), Some(y)) => x.cmp(y).then(tiebreak),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => tiebreak,
    }
}

pub(super) fn candidate_id(kind: &str, text: &str, where_str: &str) -> String {
    let payload = format!("{}|{}|{}", kind, text, where_str);
    let h = blake3::hash(payload.as_bytes()).to_hex();
    format!("c-{}", &h.to_string()[..4])
}
