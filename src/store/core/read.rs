use super::super::integrity::{check_content_hash, check_integrity_mac, check_seq_matches};
use super::super::{Store, INDEX_FILE, SCHEMA, STORE_DIR};
use crate::models::Claim;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

impl Store {
    pub fn open_or_init(cwd: &Path) -> Result<Self> {
        let root = cwd.join(STORE_DIR);
        fs::create_dir_all(&root).context("create .claims dir")?;
        let db_path = root.join(INDEX_FILE);
        let conn = Connection::open(&db_path).context("open sqlite index")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { root, conn })
    }

    pub fn claim_path(&self, seq: u64) -> PathBuf {
        self.root.join(format!("{:06}.json", seq))
    }

    /// read a claim from disk and verify both integrity layers:
    ///   1. content_hash: public fingerprint over canonical bytes (diagnostic)
    ///   2. integrity_mac: keyed MAC over the same bytes (authoritative)
    ///
    /// integrity_mac is mandatory in normal operation. legacy hash-only
    /// ledgers must be upgraded with `clms migrate-integrity`; there is no
    /// marker-controlled non-strict mode.
    pub fn read_claim(&self, seq: u64) -> Result<Claim> {
        let path = self.claim_path(seq);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("claim #{} not found at {}", seq, path.display()))?;
        let c: Claim = serde_json::from_str(&raw)?;
        let repair = std::env::var("CLAIMS_REPAIR").is_ok();
        // each gate returns Ok when the check passes OR CLAIMS_REPAIR=1
        // lets us proceed despite a failure; Err only when the gate failed
        // and repair is off. read_claim stays flat.
        check_seq_matches(&c, seq, &path, repair)?;
        check_content_hash(&c, seq, &path, repair)?;
        check_integrity_mac(&c, seq, &path, repair)?;
        Ok(c)
    }

    /// load every claim in the ledger in seq order. wraps the
    /// `all_seqs() -> map(read_claim) -> collect::<Result<Vec<_>>>()`
    /// pattern that was duplicated across output.rs, commands/suspect.rs,
    /// and elsewhere. one fail-fast `?` for any read error.
    pub fn all_claims(&self) -> Result<Vec<Claim>> {
        self.all_seqs()?
            .into_iter()
            .map(|s| self.read_claim(s))
            .collect()
    }
}
