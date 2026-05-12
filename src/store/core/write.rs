use super::super::integrity::{
    canonical_content_hash, canonical_integrity_mac, claim_seq_from_path,
    load_or_create_integrity_key, maybe_enable_integrity_strict_mode,
};
use super::super::Store;
use crate::models::Claim;
use anyhow::{Context, Result};
use rusqlite::params;
use std::fs;

impl Store {
    pub fn next_seq(&self) -> Result<u64> {
        let max: Option<i64> = self
            .conn
            .query_row("SELECT MAX(seq) FROM claims", [], |r| r.get(0))
            .unwrap_or(None);
        Ok((max.unwrap_or(0) as u64) + 1)
    }

    pub fn write_claim(&mut self, claim: &mut Claim) -> Result<()> {
        let key = load_or_create_integrity_key()?;
        claim.content_hash = Some(canonical_content_hash(claim));
        claim.integrity_mac = Some(canonical_integrity_mac(claim, &key));

        let path = self.claim_path(claim.seq);
        let pretty = serde_json::to_string_pretty(claim)?;
        fs::write(&path, pretty).with_context(|| format!("write {}", path.display()))?;

        self.index_claim(claim)?;
        maybe_enable_integrity_strict_mode(&self.root)?;
        Ok(())
    }

    fn index_claim(&mut self, c: &Claim) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO claims(seq, ulid, state, confidence, text, tags, agent, session, git_sha, created_at, updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                c.seq as i64,
                c.id.to_string(),
                c.state.as_str(),
                c.confidence().map(|t| t.as_str()),
                c.text,
                c.tags.join(","),
                c.agent,
                c.session,
                c.git_sha,
                c.created_at.to_rfc3339(),
                c.updated_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "DELETE FROM edges WHERE from_seq = ?1",
            params![c.seq as i64],
        )?;
        for e in &c.edges {
            tx.execute(
                "INSERT OR IGNORE INTO edges(from_seq, to_seq, type) VALUES(?1,?2,?3)",
                params![c.seq as i64, e.to_seq as i64, e.r#type.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reindex_all(&mut self) -> Result<usize> {
        let claims = self.collect_all_claims_sorted()?;
        self.conn
            .execute_batch("DELETE FROM edges; DELETE FROM claims;")?;
        let mut count = 0;
        for mut claim in claims {
            self.write_claim(&mut claim)?;
            count += 1;
        }
        maybe_enable_integrity_strict_mode(&self.root)?;
        Ok(count)
    }

    /// read every claim file in the ledger root and return them sorted by
    /// seq. extracted from reindex_all so the parent stays flat.
    fn collect_all_claims_sorted(&self) -> Result<Vec<Claim>> {
        let mut claims = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let seq = claim_seq_from_path(&path)?;
            claims.push(self.read_claim(seq)?);
        }
        claims.sort_by_key(|c| c.seq);
        Ok(claims)
    }
}
