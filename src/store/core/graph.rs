use super::super::Store;
use crate::models::EdgeType;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

impl Store {
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
        let mut seen = HashSet::new();
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
}
