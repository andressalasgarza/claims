use super::integrity::{
    canonical_content_hash, canonical_integrity_mac, check_content_hash,
    check_integrity_mac, check_seq_matches, claim_seq_from_path,
    load_or_create_integrity_key, maybe_enable_integrity_strict_mode,
};
use super::{Store, INDEX_FILE, SCHEMA, STORE_DIR};
use crate::models::{Claim, EdgeType};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
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

    pub fn claim_path(&self, seq: u64) -> PathBuf {
        self.root.join(format!("{:06}.json", seq))
    }

    /// read a claim from disk and verify both integrity layers:
    ///   1. content_hash: public fingerprint over canonical bytes (diagnostic)
    ///   2. integrity_mac: keyed MAC over the same bytes (authoritative)
    ///
    /// legacy ledgers may have only content_hash until they are migrated via
    /// `clms reindex`. once every file carries integrity_mac, strict mode is
    /// enabled and hash-only claims are refused.
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
        check_integrity_mac(&c, seq, &path, &self.root, repair)?;
        Ok(c)
    }

    pub fn resolve(&self, ident: &str) -> Result<u64> {
        if let Ok(seq) = ident.parse::<u64>() {
            return Ok(seq);
        }
        let row: Option<i64> = self
            .conn
            .query_row(
                "SELECT seq FROM claims WHERE ulid = ?1",
                params![ident],
                |r| r.get(0),
            )
            .ok();
        row.map(|n| n as u64)
            .ok_or_else(|| anyhow!("could not resolve '{}' to a claim id", ident))
    }

    pub fn all_seqs(&self) -> Result<Vec<u64>> {
        let mut s = self.conn.prepare("SELECT seq FROM claims ORDER BY seq")?;
        let rows = s.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.filter_map(|r| r.ok()).map(|n| n as u64).collect())
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

    pub fn dependents_of(&self, seq: u64) -> Result<Vec<(u64, String)>> {
        let mut s = self
            .conn
            .prepare("SELECT from_seq, type FROM edges WHERE to_seq = ?1")?;
        let rows = s.query_map(params![seq as i64], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn transitive_dependents(&self, seq: u64) -> Result<Vec<u64>> {
        let mut frontier = vec![seq];
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        while let Some(n) = frontier.pop() {
            for (dep, t) in self.dependents_of(n)? {
                if t != EdgeType::DependsOn.as_str() {
                    continue;
                }
                if seen.insert(dep) {
                    out.push(dep);
                    frontier.push(dep);
                }
            }
        }
        Ok(out)
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
        self.conn.execute_batch("DELETE FROM edges; DELETE FROM claims;")?;
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

