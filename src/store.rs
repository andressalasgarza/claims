use crate::models::{Claim, Edge, EdgeType, Evidence, State};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = ".claims";
pub const INDEX_FILE: &str = "index.db";

pub struct Store {
    pub root: PathBuf,
    pub conn: Connection,
}

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
        let mut to_serialize = claim.clone();
        to_serialize.content_hash = None;
        let canonical = serde_json::to_string(&to_serialize)?;
        let hash = blake3::hash(canonical.as_bytes()).to_hex().to_string();
        claim.content_hash = Some(hash);

        let path = self.claim_path(claim.seq);
        let pretty = serde_json::to_string_pretty(claim)?;
        fs::write(&path, pretty).with_context(|| format!("write {}", path.display()))?;

        self.index_claim(claim)?;
        Ok(())
    }

    pub fn claim_path(&self, seq: u64) -> PathBuf {
        self.root.join(format!("{:06}.json", seq))
    }

    pub fn read_claim(&self, seq: u64) -> Result<Claim> {
        let path = self.claim_path(seq);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("claim #{} not found at {}", seq, path.display()))?;
        let c: Claim = serde_json::from_str(&raw)?;
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
        self.conn.execute_batch(
            "DELETE FROM edges; DELETE FROM claims;",
        )?;
        let mut count = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path)?;
            let c: Claim = serde_json::from_str(&raw)?;
            self.index_claim(&c)?;
            count += 1;
        }
        Ok(count)
    }
}

/// dispatch to per-method validator. behavior preserved exactly; the only
/// change vs pre-refactor is that each per-method block is now its own fn,
/// so cyclomatic complexity is bounded per validator instead of stacking.
pub fn validate_evidence(ev: &Evidence) -> Result<()> {
    use crate::models::EvidenceMethod::*;
    match ev.method {
        PropTest => validate_prop_test(ev),
        IntegrationTest => validate_integration_test(ev),
        ReplayTest => validate_replay_test(ev),
        StatTest => validate_stat_test(ev),
        Observed => validate_observed(ev),
        Documented => validate_documented(ev),
        Derived => validate_derived(ev),
    }
}

#[inline]
fn missing_ref(ev: &Evidence) -> bool {
    ev.r#ref.is_empty()
}

#[inline]
fn missing_str(opt: &Option<String>) -> bool {
    opt.as_deref().unwrap_or("").trim().is_empty()
}

fn validate_prop_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("prop-test requires --ref <path-to-test-file>"));
    }
    if ev.exit_code.is_none() {
        return Err(anyhow!("prop-test requires --exit-code <int>"));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "prop-test requires --cmd \"<shell cmd that ran the proptest/quickcheck/fuzz>\". the cmd must be re-runnable for falsification audit."
        ));
    }
    Ok(())
}

fn validate_integration_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("integration-test requires --ref <path-to-test-file>"));
    }
    if ev.exit_code.is_none() {
        return Err(anyhow!("integration-test requires --exit-code <int>"));
    }
    if missing_str(&ev.target) {
        return Err(anyhow!(
            "integration-test requires --target <url-or-host>. the test must hit a real external system the author does not control; without --target the falsification surface is undocumented."
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "integration-test requires --cmd \"<shell cmd that probed --target>\"."
        ));
    }
    Ok(())
}

fn validate_replay_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("replay-test requires --ref <path-to-strategy-or-replay-script>"));
    }
    if ev.exit_code.is_none() {
        return Err(anyhow!("replay-test requires --exit-code <int>"));
    }
    let dataset = ev.dataset.as_deref().unwrap_or("").trim();
    if dataset.is_empty() {
        return Err(anyhow!(
            "replay-test requires --dataset <path>. the dataset must be real-world capture (not synthetic); it is content-hashed for tamper-evidence."
        ));
    }
    if ev.dataset_hash.is_none() {
        return Err(anyhow!(
            "replay-test requires the dataset at --dataset to exist as a local file at verify time so it can be content-hashed. path '{}' could not be hashed.",
            dataset
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "replay-test requires --cmd \"<shell cmd that replayed --dataset through --ref>\"."
        ));
    }
    Ok(())
}

fn validate_stat_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("stat-test requires --ref <path>"));
    }
    if ev.p_value.is_none() {
        return Err(anyhow!("stat-test requires --p-value <float>"));
    }
    if ev.sample_size.is_none() {
        return Err(anyhow!("stat-test requires --sample-size <int>"));
    }
    if ev.test_type.is_none() {
        return Err(anyhow!(
            "stat-test requires --test-type <name> (e.g. ks, t, chi2)"
        ));
    }
    if ev.data_source.is_none() {
        return Err(anyhow!(
            "stat-test requires --data-source <real|live>. simulated/synthetic data is refused: it cannot falsify a claim about reality."
        ));
    }
    Ok(())
}

fn validate_observed(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("observed requires --ref <path-url-or-hash>"));
    }
    Ok(())
}

fn validate_documented(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("documented requires --ref <url>"));
    }
    if missing_str(&ev.quote) {
        return Err(anyhow!(
            "documented requires --quote \"<exact text from doc>\""
        ));
    }
    Ok(())
}

fn validate_derived(ev: &Evidence) -> Result<()> {
    if ev.from_claims.len() < 2 {
        return Err(anyhow!(
            "derived requires at least two --from <claim_id> entries"
        ));
    }
    Ok(())
}

pub fn hash_ref_if_local(r: &str) -> Option<String> {
    let p = Path::new(r);
    if p.exists() && p.is_file() {
        let bytes = fs::read(p).ok()?;
        Some(blake3::hash(&bytes).to_hex().to_string())
    } else {
        None
    }
}

/// returns Some((prior_hash, prior_recorded_at_rfc3339)) if `claim` already has
/// evidence pointing at the same ref path with a different content hash.
pub fn detect_drift(
    claim: &Claim,
    new_ref: &str,
    new_hash: &Option<String>,
) -> Option<(String, String)> {
    let new_hash = new_hash.as_deref()?;
    for prior in &claim.evidence {
        if prior.r#ref != new_ref {
            continue;
        }
        if let Some(prior_hash) = &prior.ref_hash {
            if prior_hash != new_hash {
                return Some((prior_hash.clone(), prior.recorded_at.to_rfc3339()));
            }
        }
    }
    None
}

/// run a shell command, return (exit_code, stdout_bytes).
pub fn run_cmd(cmd: &str) -> Result<(i32, Vec<u8>)> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("failed to execute: {}", cmd))?;
    Ok((out.status.code().unwrap_or(-1), out.stdout))
}

/// find the latest evidence on a claim that has a stored cmd. used by `rerun`.
/// any method that reports `is_runnable() == true` qualifies.
pub fn latest_runnable_evidence(claim: &Claim) -> Option<&Evidence> {
    claim
        .evidence
        .iter()
        .rev()
        .find(|e| e.cmd.is_some() && e.method.is_runnable())
}

pub fn cascade_suspect(store: &mut Store, seq: u64) -> Result<Vec<u64>> {
    let dependents = store.transitive_dependents(seq)?;
    for dep in &dependents {
        let mut c = store.read_claim(*dep)?;
        if c.state == State::Verified || c.state == State::Pending {
            c.state = State::Suspect;
            c.updated_at = chrono::Utc::now();
            store.write_claim(&mut c)?;
        }
    }
    Ok(dependents)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS claims (
    seq         INTEGER PRIMARY KEY,
    ulid        TEXT NOT NULL UNIQUE,
    state       TEXT NOT NULL,
    confidence  TEXT,
    text        TEXT NOT NULL,
    tags        TEXT,
    agent       TEXT,
    session     TEXT,
    git_sha     TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_claims_state ON claims(state);
CREATE INDEX IF NOT EXISTS idx_claims_tags  ON claims(tags);

CREATE TABLE IF NOT EXISTS edges (
    from_seq INTEGER NOT NULL,
    to_seq   INTEGER NOT NULL,
    type     TEXT NOT NULL,
    PRIMARY KEY (from_seq, to_seq, type)
);
CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_seq);
"#;

pub fn _used(_: &Edge) {}
