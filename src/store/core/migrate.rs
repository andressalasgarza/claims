use super::super::integrity::{
    canonical_content_hash, canonical_integrity_mac, claim_seq_from_path,
    load_or_create_integrity_key,
};
use super::super::{IntegrityMigration, Store};
use crate::models::Claim;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

impl Store {
    pub fn migrate_integrity(&mut self) -> Result<IntegrityMigration> {
        let key = load_or_create_integrity_key()?;
        let paths = claim_json_paths(&self.root)?;
        let mut upgraded = 0usize;
        for path in &paths {
            let (mut claim, missing_mac) = read_migration_claim(path, &key)?;
            self.write_claim(&mut claim)?;
            upgraded += usize::from(missing_mac);
        }
        Ok(IntegrityMigration {
            checked: paths.len(),
            upgraded,
        })
    }
}

fn claim_json_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_migration_claim(path: &Path, key: &[u8; 32]) -> Result<(Claim, bool)> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let claim: Claim = serde_json::from_str(&raw)
        .with_context(|| format!("parse {} for integrity migration", path.display()))?;
    assert_migration_seq(&claim, path)?;
    assert_migration_content_hash(&claim, path)?;
    let missing_mac = claim.integrity_mac.is_none();
    assert_migration_existing_mac(&claim, path, key)?;
    Ok((claim, missing_mac))
}

fn assert_migration_seq(claim: &Claim, path: &Path) -> Result<()> {
    let seq = claim_seq_from_path(path)?;
    if claim.seq == seq {
        return Ok(());
    }
    Err(anyhow!(
        "cannot migrate-integrity: claim file {} declares seq={} but filename implies #{}.",
        path.display(),
        claim.seq,
        seq,
    ))
}

fn assert_migration_content_hash(claim: &Claim, path: &Path) -> Result<()> {
    let stored = claim.content_hash.as_deref().ok_or_else(|| {
        anyhow!(
            "cannot migrate-integrity: claim #{} at {} has no content_hash. legacy migration needs a valid content_hash checkpoint; hashless claims are refused.",
            claim.seq,
            path.display(),
        )
    })?;
    let recomputed = canonical_content_hash(claim);
    if stored == recomputed {
        return Ok(());
    }
    Err(anyhow!(
        "cannot migrate-integrity: claim #{} content_hash mismatch.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\nlegacy migration only signs files that still match their existing content_hash checkpoint.",
        claim.seq,
        &stored[..16.min(stored.len())],
        &recomputed[..16],
        path.display(),
    ))
}

fn assert_migration_existing_mac(claim: &Claim, path: &Path, key: &[u8; 32]) -> Result<()> {
    let Some(stored) = claim.integrity_mac.as_deref() else {
        return Ok(());
    };
    let recomputed = canonical_integrity_mac(claim, key);
    if stored == recomputed {
        return Ok(());
    }
    Err(anyhow!(
        "cannot migrate-integrity: claim #{} already has an integrity_mac, but it does not verify with the current key.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\nrestore the original integrity key or inspect with CLAIMS_REPAIR=1 before migrating.",
        claim.seq,
        &stored[..16.min(stored.len())],
        &recomputed[..16],
        path.display(),
    ))
}
