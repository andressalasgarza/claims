use super::super::INTEGRITY_STRICT_MARKER;
use crate::models::Claim;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

fn integrity_strict_marker_path(root: &Path) -> PathBuf {
    root.join(INTEGRITY_STRICT_MARKER)
}

pub(crate) fn integrity_strict_mode(root: &Path) -> bool {
    integrity_strict_marker_path(root).exists()
}

/// scan the ledger and return true iff every claim file carries an
/// integrity_mac. used as the precondition for entering strict-integrity
/// mode: we only flip the strict marker on once the migration is complete.
fn all_claims_have_macs(root: &Path) -> Result<bool> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        let claim: Claim = serde_json::from_str(&raw)
            .with_context(|| format!("parse {} while checking strict-integrity readiness", path.display()))?;
        if claim.integrity_mac.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn maybe_enable_integrity_strict_mode(root: &Path) -> Result<()> {
    if integrity_strict_mode(root) {
        return Ok(());
    }
    if !all_claims_have_macs(root)? {
        return Ok(());
    }
    fs::write(integrity_strict_marker_path(root), "strict\n")
        .context("write strict-integrity marker")?;
    Ok(())
}

pub(crate) fn claim_seq_from_path(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("unexpected claim filename {}", path.display()))?;
    stem.parse::<u64>()
        .map_err(|_| anyhow!("unexpected claim filename {}", path.display()))
}

