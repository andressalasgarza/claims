use super::build::build_pending_claim;
use super::CommitOutcome;
use crate::store::Store;
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::Path;

pub(super) fn process_candidates(
    store: &mut Store,
    candidates: &[serde_json::Value],
    session: &str,
    archaeology_dir: &Path,
    existing_ids: &HashSet<String>,
) -> Result<CommitOutcome> {
    let mut out = CommitOutcome { written: 0, skipped: 0, summary: Vec::new() };
    for c in candidates {
        if !c["keep"].as_bool().unwrap_or(false) {
            continue;
        }
        let cid = c["id"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate missing id"))?
            .to_string();
        if existing_ids.contains(&cid) {
            out.skipped += 1;
            continue;
        }
        let mut claim = build_pending_claim(store, c, session, archaeology_dir)?;
        store.write_claim(&mut claim)?;
        out.written += 1;
        out.summary.push(serde_json::json!({
            "candidate_id": cid,
            "text": c["text"],
        }));
    }
    Ok(out)
}

pub(super) fn scan_existing_candidate_ids(store: &Store) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for seq in store.all_seqs()? {
        let claim = match store.read_claim(seq) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(meta) = &claim.archaeology_meta {
            out.insert(meta.candidate_id.clone());
        }
    }
    Ok(out)
}

