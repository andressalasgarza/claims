use super::super::Store;
use anyhow::{anyhow, Result};
use rusqlite::params;

impl Store {
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
}
