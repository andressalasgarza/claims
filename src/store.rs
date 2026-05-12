use crate::models::{BenchmarkMetric, Claim, EdgeType, Evidence, State};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = ".claims";
pub const INDEX_FILE: &str = "index.db";
const INTEGRITY_STRICT_MARKER: &str = ".integrity.strict";
const DEFAULT_INTEGRITY_KEY_DIR: &str = ".clms";
const DEFAULT_INTEGRITY_KEY_FILE: &str = "integrity.key";

fn canonical_claim_bytes(claim: &Claim) -> Vec<u8> {
    let mut to_serialize = claim.clone();
    to_serialize.content_hash = None;
    to_serialize.integrity_mac = None;
    serde_json::to_vec(&to_serialize)
        .expect("Claim serializes (Serialize is derived)")
}

/// canonical content hash: serialize the claim with integrity fields blanked,
/// then blake3. this remains a cheap public fingerprint for diagnostics and
/// drift reporting, but it is NOT the authority for tamper-resistance.
fn canonical_content_hash(claim: &Claim) -> String {
    blake3::hash(&canonical_claim_bytes(claim)).to_hex().to_string()
}

/// keyed MAC over the canonical claim bytes. unlike content_hash, this cannot
/// be forged by someone who can merely edit the json file; they also need the
/// signing key material.
fn canonical_integrity_mac(claim: &Claim, key: &[u8; 32]) -> String {
    blake3::keyed_hash(key, &canonical_claim_bytes(claim))
        .to_hex()
        .to_string()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn decode_hex_32(s: &str, label: &str) -> Result<[u8; 32]> {
    let hex = s.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "{} must be exactly 64 hex chars (32 bytes)",
            label
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("{} contains invalid hex", label))?;
    }
    Ok(out)
}

fn default_integrity_key_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        anyhow!(
            "cannot locate HOME for default integrity key path. set CLAIMS_INTEGRITY_KEY_FILE or CLAIMS_INTEGRITY_KEY_HEX."
        )
    })?;
    Ok(PathBuf::from(home)
        .join(DEFAULT_INTEGRITY_KEY_DIR)
        .join(DEFAULT_INTEGRITY_KEY_FILE))
}

fn integrity_key_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CLAIMS_INTEGRITY_KEY_FILE") {
        return Ok(PathBuf::from(path));
    }
    default_integrity_key_path()
}

fn load_integrity_key() -> Result<[u8; 32]> {
    if let Ok(hex) = std::env::var("CLAIMS_INTEGRITY_KEY_HEX") {
        return decode_hex_32(&hex, "CLAIMS_INTEGRITY_KEY_HEX");
    }
    let path = integrity_key_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read integrity key {}", path.display()))?;
    decode_hex_32(&raw, &format!("integrity key file {}", path.display()))
}

fn load_or_create_integrity_key() -> Result<[u8; 32]> {
    if let Ok(hex) = std::env::var("CLAIMS_INTEGRITY_KEY_HEX") {
        return decode_hex_32(&hex, "CLAIMS_INTEGRITY_KEY_HEX");
    }
    let path = integrity_key_path()?;
    if path.exists() {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read integrity key {}", path.display()))?;
        return decode_hex_32(&raw, &format!("integrity key file {}", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create integrity key dir {}", parent.display()))?;
    }
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| anyhow!("generate integrity key: {}", e))?;
    fs::write(&path, format!("{}\n", encode_hex(&key)))
        .with_context(|| format!("write integrity key {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {}", path.display()))?;
    }
    Ok(key)
}

fn integrity_strict_marker_path(root: &Path) -> PathBuf {
    root.join(INTEGRITY_STRICT_MARKER)
}

fn integrity_strict_mode(root: &Path) -> bool {
    integrity_strict_marker_path(root).exists()
}

fn maybe_enable_integrity_strict_mode(root: &Path) -> Result<()> {
    if integrity_strict_mode(root) {
        return Ok(());
    }
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
            return Ok(());
        }
    }
    fs::write(integrity_strict_marker_path(root), "strict\n")
        .context("write strict-integrity marker")?;
    Ok(())
}

fn claim_seq_from_path(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("unexpected claim filename {}", path.display()))?;
    stem.parse::<u64>()
        .map_err(|_| anyhow!("unexpected claim filename {}", path.display()))
}

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
        if c.seq != seq {
            if repair {
                return Ok(c);
            }
            return Err(anyhow!(
                "claim file {} declares seq={} but was loaded as #{}. filename and payload must agree.",
                path.display(),
                c.seq,
                seq,
            ));
        }
        match c.content_hash.as_deref() {
            None => {
                if repair {
                    return Ok(c);
                }
                return Err(anyhow!(
                    "claim #{} at {} has no content_hash. either it was hand-written outside `clms`, \
or it predates the integrity field. set CLAIMS_REPAIR=1 to read anyway, then re-save with `clms reindex`.",
                    seq,
                    path.display(),
                ));
            }
            Some(stored) => {
                let recomputed = canonical_content_hash(&c);
                if stored != recomputed {
                    if repair {
                        return Ok(c);
                    }
                    return Err(anyhow!(
                        "claim #{} content_hash mismatch — file has been tampered.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\nthe json on disk does not match the hash it claims. someone edited the file outside `clms`. \
set CLAIMS_REPAIR=1 to read anyway (use only if you accept the tampered content), then re-save.",
                        seq,
                        &stored[..16.min(stored.len())],
                        &recomputed[..16],
                        path.display(),
                    ));
                }
            }
        }
        match c.integrity_mac.as_deref() {
            Some(stored) => {
                let key = match load_integrity_key() {
                    Ok(key) => key,
                    Err(err) => {
                        if repair {
                            return Ok(c);
                        }
                        return Err(anyhow!(
                            "claim #{} at {} carries integrity_mac, but the verification key is unavailable: {}. set CLAIMS_REPAIR=1 for forensic read-only access, or restore the key before using this ledger.",
                            seq,
                            path.display(),
                            err,
                        ));
                    }
                };
                let recomputed = canonical_integrity_mac(&c, &key);
                if stored != recomputed {
                    if repair {
                        return Ok(c);
                    }
                    return Err(anyhow!(
                        "claim #{} integrity_mac mismatch — file has been tampered or the wrong integrity key is in use.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\ncontent_hash alone is forgeable. integrity_mac is the authoritative check. set CLAIMS_REPAIR=1 only for forensic read-only recovery.",
                        seq,
                        &stored[..16.min(stored.len())],
                        &recomputed[..16],
                        path.display(),
                    ));
                }
            }
            None => {
                if integrity_strict_mode(&self.root) {
                    if repair {
                        return Ok(c);
                    }
                    return Err(anyhow!(
                        "claim #{} at {} has no integrity_mac, but this ledger is in strict integrity mode. migrate legacy hash-only claims with `clms reindex`, or set CLAIMS_REPAIR=1 for forensic read-only access.",
                        seq,
                        path.display(),
                    ));
                }
            }
        }
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

        self.conn.execute_batch("DELETE FROM edges; DELETE FROM claims;")?;
        let mut count = 0;
        for mut claim in claims {
            self.write_claim(&mut claim)?;
            count += 1;
        }
        maybe_enable_integrity_strict_mode(&self.root)?;
        Ok(count)
    }
}

/// dispatch to per-method validator. behavior preserved exactly; the only
/// change vs pre-refactor is that each per-method block is now its own fn,
/// so cyclomatic complexity is bounded per validator instead of stacking.
pub fn validate_evidence(ev: &Evidence, store: &Store, current_seq: u64) -> Result<()> {
    use crate::models::EvidenceMethod::*;
    match ev.method {
        PropTest => validate_prop_test(ev),
        IntegrationTest => validate_integration_test(ev),
        ReplayTest => validate_replay_test(ev),
        StatTest => validate_stat_test(ev),
        Benchmark => validate_benchmark(ev),
        Estimate => validate_estimate(ev),
        Observed => validate_observed(ev),
        Documented => validate_documented(ev),
        Derived => validate_derived(ev, store, current_seq),
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

// ---------- per-field validation helpers ----------
//
// each helper bundles two or more inline checks (presence + shape) into a
// single call site. drops caller ccn by (n_internal_decisions - 1) per use
// without changing user-visible error wording: the caller still owns the
// error message via closures, so the golden file stays byte-identical.

/// the artifact-driven preamble shared by stat-test/benchmark/estimate:
/// --ref must be present, must hash as a local file, and --cmd must be
/// non-empty. centralizing the trio drops 3 ccn from each validate_* that
/// uses it.
fn req_artifact_inputs(
    ev: &Evidence, missing_ref_msg: &'static str, missing_local_msg: &'static str,
    missing_cmd_msg: &'static str,
) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("{}", missing_ref_msg));
    }
    if ev.ref_hash.is_none() {
        return Err(anyhow!("{}", missing_local_msg));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!("{}", missing_cmd_msg));
    }
    Ok(())
}

/// present-and-finite: bundles `.ok_or_else()? + !is_finite()` into one
/// caller decision. each use saves 1 ccn vs inline.
fn req_finite_field(
    opt: Option<f64>, miss_err: impl FnOnce() -> anyhow::Error,
    nonfinite_err: impl FnOnce(f64) -> anyhow::Error,
) -> Result<f64> {
    let v = opt.ok_or_else(miss_err)?;
    if !v.is_finite() {
        return Err(nonfinite_err(v));
    }
    Ok(v)
}

/// present-and-at-least: bundles `.ok_or_else()? + n < lower` into one
/// caller decision. used for sample_size >= 2 across all 3 artifact methods.
fn req_u64_at_least(
    opt: Option<u64>, lower: u64, miss_err: impl FnOnce() -> anyhow::Error,
    too_low_err: impl FnOnce(u64) -> anyhow::Error,
) -> Result<u64> {
    let n = opt.ok_or_else(miss_err)?;
    if n < lower {
        return Err(too_low_err(n));
    }
    Ok(n)
}

/// some-or-else for non-numeric option fields (estimator, test_type,
/// data_source). just a thin alias to make call sites uniform.
fn req_present<T>(opt: Option<T>, err: impl FnOnce() -> anyhow::Error) -> Result<T> {
    opt.ok_or_else(err)
}

fn validate_prop_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("prop-test requires --ref <path-to-test-file>"));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "prop-test requires --cmd \"<shell cmd that ran the proptest/quickcheck/fuzz>\". the cmd is executed at verify time and the actual exit_code is captured."
        ));
    }
    Ok(())
}

fn validate_integration_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("integration-test requires --ref <path-to-test-file>"));
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

/// direction-aware threshold check for benchmark. extracted to keep the
/// main validator flat: this captures the (metric, value, threshold) triple
/// + the higher/lower-better routing as a single decision in the caller.
fn benchmark_check_threshold(metric: BenchmarkMetric, v: f64, t: f64) -> Result<()> {
    let passes = if metric.is_higher_better() { v >= t } else { v <= t };
    if passes {
        return Ok(());
    }
    let dir = if metric.is_higher_better() {
        "higher-better, need value >= threshold"
    } else {
        "lower-better, need value <= threshold"
    };
    Err(anyhow!(
        "benchmark refused: metric_value={} did not meet threshold={} for metric '{}' ({}).\nstate stays pending; either fix the eval, lower the threshold honestly, or write a different claim if the miss is the actual finding.",
        v, t, metric.as_str(), dir,
    ))
}

/// pull required numeric/enum fields from a benchmark evidence record.
/// returns the validated (metric, metric_value, threshold) triple. extracted
/// to keep validate_benchmark's body flat; closures inside this fn count
/// toward this fn's ccn instead of the parent's.
fn benchmark_collect_required(ev: &Evidence) -> Result<(BenchmarkMetric, f64, f64)> {
    let metric = req_present(ev.metric, || anyhow!(
        "benchmark artifact missing 'metric'. valid: auc-roc | auc-pr | f1 | precision | recall | accuracy | balanced-accuracy | mcc | kappa-cohen | r2 | log-loss | brier | rmse | mae | mape."
    ))?;
    let v = req_finite_field(
        ev.metric_value,
        || anyhow!("benchmark artifact missing 'metric_value'"),
        |x| anyhow!(
            "benchmark --metric-value must be finite (got {}). NaN/inf are not measurable values.",
            x
        ),
    )?;
    let t = req_finite_field(
        ev.threshold,
        || anyhow!("benchmark requires --threshold <float>"),
        |x| anyhow!(
            "benchmark --threshold must be finite (got {}). NaN/inf are not measurable thresholds.",
            x
        ),
    )?;
    Ok((metric, v, t))
}

/// sample size + data-source presence for benchmark. extracted for the
/// same reason as benchmark_collect_required.
fn benchmark_check_meta(ev: &Evidence) -> Result<()> {
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("benchmark artifact missing 'sample_size'"),
        |n| anyhow!(
            "benchmark --sample-size must be >= 2 (got {}). a metric computed on one sample is not measurable evidence.",
            n
        ),
    )?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "benchmark artifact missing 'data_source' (expected real|live). synthetic eval sets cannot falsify a claim about real-world performance."
    ))?;
    Ok(())
}

fn validate_benchmark(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "benchmark requires --ref <path-to-local-json-artifact>",
        "benchmark requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "benchmark requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time, hashes stdout, then parses metric/sample metadata from the artifact instead of trusting cli flags.",
    )?;
    let (metric, v, t) = benchmark_collect_required(ev)?;
    benchmark_check_meta(ev)?;
    benchmark_check_threshold(metric, v, t)
}

/// CI shape checks for estimate: lo and hi must be finite, lo ≤ hi, and
/// the point estimate must lie within [lo, hi]. extracted so the parent
/// stays flat. uses three sequential checks; caller pays 1 ccn for the
/// whole CI block.
fn estimate_check_ci(point: f64, lo: f64, hi: f64) -> Result<()> {
    if !lo.is_finite() || !hi.is_finite() {
        return Err(anyhow!(
            "estimate CI bounds must be finite (got ci_lower={}, ci_upper={}).",
            lo, hi
        ));
    }
    if lo > hi {
        return Err(anyhow!(
            "estimate --ci-lower={} > --ci-upper={}. lower bound must be ≤ upper bound.",
            lo, hi
        ));
    }
    if point < lo || point > hi {
        return Err(anyhow!(
            "estimate --point-value={} is outside the declared CI [{}, {}]. the point estimate must lie within its own interval; otherwise the CI is misreported.",
            point, lo, hi
        ));
    }
    Ok(())
}

/// confidence-level must be a probability strictly between 0 and 1.
/// extracted so the parent doesn't accumulate the 3-way conjunction.
fn estimate_check_confidence(conf: f64) -> Result<()> {
    if conf.is_finite() && conf > 0.0 && conf < 1.0 {
        return Ok(());
    }
    Err(anyhow!(
        "estimate --confidence-level must be in (0, 1) (got {}). typical values: 0.90, 0.95, 0.99.",
        conf
    ))
}

/// collect estimator presence + point_value + ci bounds. closures pile up
/// here instead of the parent.
fn estimate_collect_point_and_ci(ev: &Evidence) -> Result<(f64, f64, f64)> {
    req_present(ev.estimator, || anyhow!(
        "estimate artifact missing 'estimator'. valid: mean | median | geometric-mean | std-dev | std-error | variance | skewness | kurtosis | cohens-d | odds-ratio | risk-ratio | correlation | spearman-rho."
    ))?;
    let point = req_finite_field(
        ev.point_value,
        || anyhow!("estimate artifact missing 'point_value'"),
        |x| anyhow!(
            "estimate --point-value must be finite (got {}). NaN/inf are not measurable values.",
            x
        ),
    )?;
    let lo = req_present(ev.ci_lower, || anyhow!("estimate artifact missing 'ci_lower'"))?;
    let hi = req_present(ev.ci_upper, || anyhow!("estimate artifact missing 'ci_upper'"))?;
    Ok((point, lo, hi))
}

/// confidence-level + sample-size + data-source for estimate.
fn estimate_check_meta(ev: &Evidence) -> Result<()> {
    let conf = req_present(
        ev.confidence_level,
        || anyhow!("estimate artifact missing 'confidence_level'"),
    )?;
    estimate_check_confidence(conf)?;
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("estimate artifact missing 'sample_size'"),
        |n| anyhow!(
            "estimate --sample-size must be >= 2 (got {}). a CI from a single sample is not measurable.",
            n
        ),
    )?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "estimate artifact missing 'data_source' (expected real|live). estimators on synthetic data only describe the simulator (circular)."
    ))?;
    Ok(())
}

fn validate_estimate(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "estimate requires --ref <path-to-local-json-artifact>",
        "estimate requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "estimate requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time, hashes stdout, then parses estimator/ci metadata from the artifact instead of trusting cli flags.",
    )?;
    let (point, lo, hi) = estimate_collect_point_and_ci(ev)?;
    estimate_check_ci(point, lo, hi)?;
    estimate_check_meta(ev)
}

/// p_value must be a probability AND not NaN. nan check is separate from
/// the range check because nan compares false to both bounds and would slip
/// through `contains()`.
fn stat_test_check_p_value(p: f64) -> Result<()> {
    if (0.0..=1.0).contains(&p) && !p.is_nan() {
        return Ok(());
    }
    Err(anyhow!(
        "stat-test --p-value must be in [0.0, 1.0] (got {}). a p-value is a probability — outside that range it is not a p-value.",
        p
    ))
}

/// sample-size + test_type + data-source for stat-test. extracted so the
/// closure cost doesn't pile onto validate_stat_test.
fn stat_test_check_meta(ev: &Evidence) -> Result<()> {
    req_u64_at_least(
        ev.sample_size,
        2,
        || anyhow!("stat-test artifact missing 'sample_size'"),
        |n| anyhow!(
            "stat-test --sample-size must be >= 2 (got {}). a single sample (or zero) cannot falsify a distributional claim.",
            n
        ),
    )?;
    req_present(ev.test_type, || anyhow!("stat-test artifact missing 'test_type'"))?;
    req_present(ev.data_source.as_ref(), || anyhow!(
        "stat-test artifact missing 'data_source' (expected real|live). simulated/synthetic data is refused: it cannot falsify a claim about reality."
    ))?;
    Ok(())
}

fn validate_stat_test(ev: &Evidence) -> Result<()> {
    req_artifact_inputs(
        ev,
        "stat-test requires --ref <path-to-local-json-artifact>",
        "stat-test requires --ref to exist as a local file after --cmd runs. clms hashes and parses that artifact; URLs and bare strings are not accepted.",
        "stat-test requires --cmd \"<shell cmd that produces the local json artifact at --ref>\". clms executes it at verify time and parses p_value/sample metadata from the artifact instead of trusting cli flags.",
    )?;
    let p = req_present(ev.p_value, || anyhow!("stat-test artifact missing 'p_value'"))?;
    stat_test_check_p_value(p)?;
    stat_test_check_meta(ev)
}

/// observed --ref must be one of:
///   - an existing local file (hashable; the H17 path will content_hash it)
///   - a URL with a recognized scheme (http/https/file/ftp/ssh/git)
///   - a content-address: sha256:HEX / blake3:HEX / sha1:HEX (any hex case)
///
/// random strings like "foo" or "my notes" are refused. without this check
/// observed degrades to "agent typed any string" and provides no auditable
/// surface for refutation.
pub fn ref_shape_acceptable(s: &str) -> bool {
    if std::path::Path::new(s).exists() {
        return true;
    }
    let recognized_url_schemes = [
        "http://", "https://", "file://", "ftp://", "ftps://", "ssh://", "git://", "ws://", "wss://",
    ];
    if recognized_url_schemes.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // content-address: <hashname>:<hex>
    if let Some((scheme, hex)) = s.split_once(':') {
        if matches!(scheme, "sha256" | "sha1" | "sha512" | "blake3" | "md5")
            && !hex.is_empty()
            && hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            return true;
        }
    }
    false
}

fn validate_observed(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("observed requires --ref <path-url-or-hash>"));
    }
    if !ref_shape_acceptable(&ev.r#ref) {
        return Err(anyhow!(
            "observed --ref '{}' is not auditable. it must be one of: (a) an existing local file path, (b) a URL (http/https/file/ftp/ssh/git/ws), or (c) a content-address (sha256:HEX / blake3:HEX / sha1:HEX). bare strings are refused because they provide no surface for refutation.",
            ev.r#ref
        ));
    }
    Ok(())
}

/// is this --target a loopback or RFC1918 / link-local / unspecified address?
/// rejects: localhost, 127/8, 0/8, 10/8, 172.16/12, 192.168/16, 169.254/16,
/// ::1, fc00::/7, fe80::/10. matches by hostname *or* parsed IP literal so
/// both `https://localhost:8080/health` and `http://127.0.0.1` are caught.
pub fn target_is_local(target: &str) -> bool {
    // strip scheme + path: "https://host:port/path" -> "host:port" -> "host"
    let after_scheme = target.split_once("://").map(|(_, r)| r).unwrap_or(target);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = match host_port.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host_port,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "localhost.localdomain" | "" | "::" | "::1"
    ) {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.octets()[0] == 0
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || {
                    let s = v6.segments()[0];
                    // fc00::/7 (unique-local) or fe80::/10 (link-local)
                    (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
                }
            }
        };
    }
    false
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

/// derived parent validation. before this commit, the only check was
/// `from_claims.len() >= 2`. that left several trivial bypasses in production:
/// same id twice, non-existent parents, unverified parents (PENDING),
/// refuted parents, and self-derivation (cite yourself to verify yourself).
/// all now refuse with specific messages. derived must compose only over
/// claims that have actually crossed the falsifiability bar.
/// derived-input gates: arity, self-reference, dedup. flat sequence so a
/// future maintainer adding another "input shape" check has one obvious
/// home. saves 3 ccn from the parent.
fn derived_check_inputs(ev: &Evidence, current_seq: u64) -> Result<()> {
    if ev.from_claims.len() < 2 {
        return Err(anyhow!(
            "derived requires at least two --from <claim_id> entries"
        ));
    }
    if ev.from_claims.contains(&current_seq) {
        return Err(anyhow!(
            "derived: claim #{} cannot cite itself in --from. self-derivation is circular.",
            current_seq
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for id in &ev.from_claims {
        if !seen.insert(*id) {
            return Err(anyhow!(
                "derived: --from #{} listed twice. each parent claim must be unique.",
                id
            ));
        }
    }
    Ok(())
}

/// every parent must exist and be in Verified state. extracted so the
/// parent function carries one decision (the for loop) instead of three.
fn derived_check_parents(store: &Store, ev: &Evidence) -> Result<()> {
    for id in &ev.from_claims {
        let parent = store.read_claim(*id).map_err(|_| {
            anyhow!(
                "derived: --from #{} does not exist. cannot derive from a non-existent claim.",
                id
            )
        })?;
        if !matches!(parent.state, State::Verified) {
            return Err(anyhow!(
                "derived: --from #{} is {} (not verified). derived claims must compose only over verified parents — deriving from {} is unsound.",
                id,
                parent.state.as_str(),
                parent.state.as_str()
            ));
        }
    }
    Ok(())
}

/// cycle detection: walking from each --from parent, the current claim must
/// not be reachable through any chain of derived edges. without this, X can
/// derive from {Y, Z} while Y derives from {X, Z} — both verify, and the
/// "derived" relation is no longer a DAG. the falsifiability story collapses
/// because each claim's evidence ultimately rests on its own conclusion.
fn derived_check_dag(store: &Store, ev: &Evidence, current_seq: u64) -> Result<()> {
    let mut visited = std::collections::HashSet::new();
    for parent_id in &ev.from_claims {
        if let Some(cycle_path) = derived_cycle_path(store, *parent_id, current_seq, &mut visited)? {
            // cycle_path is [parent, ..., target]; prepend current_seq for the
            // human-readable round trip current_seq → parent → … → current_seq.
            let path_str = std::iter::once(current_seq)
                .chain(cycle_path.into_iter())
                .map(|s| format!("#{}", s))
                .collect::<Vec<_>>()
                .join(" → ");
            return Err(anyhow!(
                "derived: cycle detected. claim #{} would derive from #{}, but #{} is already (transitively) derived from #{}.\n  cycle: {}\n\nderived edges must form a DAG. break the cycle by refuting one of the participants or by re-grounding it in non-derived evidence.",
                current_seq, parent_id, parent_id, current_seq,
                path_str,
            ));
        }
    }
    Ok(())
}

fn validate_derived(ev: &Evidence, store: &Store, current_seq: u64) -> Result<()> {
    derived_check_inputs(ev, current_seq)?;
    derived_check_parents(store, ev)?;
    derived_check_dag(store, ev, current_seq)?;
    Ok(())
}

/// DFS from `start` looking for `target`. returns Some(path-from-start-to-target)
/// if a cycle exists, None otherwise. `visited` is shared across calls so we
/// only walk each parent once per validate_derived call (linear in graph size).
fn derived_cycle_path(
    store: &Store,
    start: u64,
    target: u64,
    visited: &mut std::collections::HashSet<u64>,
) -> Result<Option<Vec<u64>>> {
    if start == target {
        return Ok(Some(vec![start]));
    }
    if !visited.insert(start) {
        return Ok(None);
    }
    let claim = match store.read_claim(start) {
        Ok(c) => c,
        Err(_) => return Ok(None), // missing parent already rejected upstream
    };
    for ev in &claim.evidence {
        if !matches!(ev.method, crate::models::EvidenceMethod::Derived) {
            continue;
        }
        for next in &ev.from_claims {
            if let Some(mut sub) = derived_cycle_path(store, *next, target, visited)? {
                sub.insert(0, start);
                return Ok(Some(sub));
            }
        }
    }
    Ok(None)
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

/// canonical hash of an Evidence record's *user-supplied* fields. used for
/// dedup at append time. the auto-captured fields (recorded_at, ref_hash,
/// stdout_hash, dataset_hash, cmd_hash) are blanked before hashing so two
/// identical user invocations collapse to the same key, but a rerun that
/// captures a different exit_code stays distinct.
pub fn evidence_user_hash(ev: &Evidence) -> String {
    let mut canon = ev.clone();
    canon.recorded_at = chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap_or_else(chrono::Utc::now);
    canon.ref_hash = None;
    canon.stdout_hash = None;
    canon.dataset_hash = None;
    canon.cmd_hash = None;
    let s = serde_json::to_string(&canon).unwrap_or_default();
    blake3::hash(s.as_bytes()).to_hex().to_string()
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

/// returns Some((prior_ref, prior_recorded_at_rfc3339)) if `claim` has prior
/// evidence whose ref_hash equals the new ref_hash but whose ref *string*
/// differs. this is the rename-pattern dodge: agent copies test.py to
/// test_v2.py (or moves it, or symlinks), then verifies the new path as if it
/// were a distinct piece of evidence. without this check, dedup (which keys on
/// ref string) accepts the duplicate-content-under-different-name as
/// independent evidence, inflating the evidence count for free.
///
/// pairs with detect_drift (same ref + different hash) and detect_dataset_drift
/// (same dataset path + different hash). together they close the rename and
/// content-swap branches of evidence-laundering.
pub fn detect_rename(
    claim: &Claim,
    new_ref: &str,
    new_hash: &Option<String>,
) -> Option<(String, String)> {
    let new_hash = new_hash.as_deref()?;
    for prior in &claim.evidence {
        if prior.r#ref == new_ref {
            continue; // same ref → handled by detect_drift
        }
        if let Some(prior_hash) = &prior.ref_hash {
            if prior_hash == new_hash {
                return Some((prior.r#ref.clone(), prior.recorded_at.to_rfc3339()));
            }
        }
    }
    None
}

/// returns Some((prior_hash, prior_recorded_at_rfc3339)) if `claim` already has
/// replay-test evidence with the same `dataset` path but a different content
/// hash. without this check, an agent can swap the bytes of a parquet file
/// (or any other dataset) between two `replay-test` verifies and clms records
/// both as if the underlying data hadn't changed. detect_drift only catches
/// drift on the ref string; this catches drift on the dataset payload.
pub fn detect_dataset_drift(
    claim: &Claim,
    new_dataset: Option<&str>,
    new_dataset_hash: &Option<String>,
) -> Option<(String, String, String)> {
    let new_dataset = new_dataset?;
    let new_hash = new_dataset_hash.as_deref()?;
    for prior in &claim.evidence {
        let prior_ds = match prior.dataset.as_deref() {
            Some(s) if s == new_dataset => s,
            _ => continue,
        };
        if let Some(prior_hash) = &prior.dataset_hash {
            if prior_hash != new_hash {
                return Some((
                    prior_ds.to_string(),
                    prior_hash.clone(),
                    prior.recorded_at.to_rfc3339(),
                ));
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

/// flip transitive dependents of `seq` from Verified|Pending to Suspect.
/// returns the seqs that were actually flipped (not all dependents). callers
/// rely on the returned vec to render an honest cascade message; previously
/// this returned every dependent regardless of whether it changed state, which
/// caused `clms refute --cascade` to lie:
///
///   cascade: 1 dependent claim(s) auto-flagged suspect: [42]
///
/// where claim 42 was Unverifiable and stayed Unverifiable. the message
/// claimed an action that did not happen. now the message reports only
/// claims whose state actually changed.
pub fn cascade_suspect(store: &mut Store, seq: u64) -> Result<Vec<u64>> {
    let dependents = store.transitive_dependents(seq)?;
    let mut flipped = Vec::new();
    for dep in &dependents {
        let mut c = store.read_claim(*dep)?;
        if matches!(c.state, State::Verified | State::Pending) {
            c.state = State::Suspect;
            c.updated_at = chrono::Utc::now();
            store.write_claim(&mut c)?;
            flipped.push(*dep);
        }
    }
    Ok(flipped)
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
