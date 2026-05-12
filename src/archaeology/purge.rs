use super::{PurgeArgs, AGENT};
use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub(super) fn cmd_purge(store: &mut Store, args: PurgeArgs, fmt: OutputFormat) -> Result<()> {
    let target_agent = args.agent.unwrap_or_else(|| AGENT.to_string());
    let target_session = args.session;
    let victims = collect_purge_victims(store, &target_agent, &target_session)?;
    let removed = delete_claim_files(&victims)?;
    let reindexed = store.reindex_all()?;
    render_purge_result(fmt, &target_agent, &target_session, &victims, removed, reindexed);
    Ok(())
}

fn collect_purge_victims(store: &Store, target_agent: &str, target_session: &str) -> Result<Vec<u64>> {
    let mut victims = Vec::new();
    for seq in store.all_seqs()? {
        let claim = match store.read_claim(seq) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let agent_match = claim.agent.as_deref() == Some(target_agent);
        let session_match = claim.session.as_deref() == Some(target_session);
        if agent_match && session_match {
            victims.push(seq);
        }
    }
    Ok(victims)
}

fn delete_claim_files(victims: &[u64]) -> Result<usize> {
    let mut removed = 0usize;
    for seq in victims {
        let path = PathBuf::from(".claims").join(format!("{:06}.json", seq));
        if !path.exists() {
            continue;
        }
        fs::remove_file(&path)
            .with_context(|| format!("remove {}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

fn render_purge_result(
    fmt: OutputFormat, target_agent: &str, target_session: &str,
    victims: &[u64], removed: usize, reindexed: usize,
) {
    if matches!(fmt, OutputFormat::Ai) {
        println!(
            "{}",
            serde_json::json!({
                "agent": target_agent,
                "session": target_session,
                "matched_seqs": victims,
                "removed_files": removed,
                "reindexed_total": reindexed,
            })
        );
        return;
    }
    if victims.is_empty() {
        println!(
            "no claims matched agent={} session={}",
            target_agent, target_session
        );
        return;
    }
    println!(
        "purged {} claims (seqs: {:?}) for agent={} session={}",
        removed, victims, target_agent, target_session
    );
    println!("reindexed {} remaining claims", reindexed);
}
