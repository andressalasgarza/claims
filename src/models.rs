use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub const SCHEMA_VERSION: &str = "1.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Pending,
    Verified,
    Refuted,
    Unverifiable,
    Suspect,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Verified => "verified",
            State::Refuted => "refuted",
            State::Unverifiable => "unverifiable",
            State::Suspect => "suspect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfidenceTier {
    Derived = 1,
    Documented = 2,
    Observed = 3,
    Empirical = 4,
}

impl ConfidenceTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfidenceTier::Empirical => "empirical",
            ConfidenceTier::Observed => "observed",
            ConfidenceTier::Documented => "documented",
            ConfidenceTier::Derived => "derived",
        }
    }
}

/// every method has a falsification surface — a data source the author does not
/// fully control. `unit-test` and `sim-test` were intentionally excluded: both are
/// confirmatory by construction (you pick the input AND the expected output, or
/// you generate data from the same assumptions you're testing). they cannot
/// disagree with you, so they cannot serve as evidence in a falsifiable ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceMethod {
    /// property-based / fuzz test. randomized input generator explores input space;
    /// any failing case falsifies the property. requires --ref --exit-code --cmd.
    PropTest,
    /// hits a real external system (api/db/host) the author does not control.
    /// requires --ref --exit-code --target --cmd.
    IntegrationTest,
    /// runs code against frozen captured real-world data (not synthetic).
    /// requires --ref --exit-code --dataset --cmd.
    ReplayTest,
    /// statistical hypothesis test on real or live data. simulated data refused.
    /// requires --ref --p-value --sample-size --test-type --data-source.
    StatTest,
    /// captured runtime artifact (tx hash, log line, response). requires --ref.
    Observed,
    /// primary-source documentation + exact quote. requires --ref --quote.
    Documented,
    /// inference from at least two other claims. requires --from N (>=2).
    Derived,
}

/// for `stat-test`. simulated data is refused; the data must come from a source
/// the author does not control (real captured samples or live measurement).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataSource {
    /// real captured samples (e.g. a parquet file of past binance ticks).
    Real,
    /// live measurement at verify time (e.g. polling a real api right now).
    Live,
}

impl DataSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataSource::Real => "real",
            DataSource::Live => "live",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "real" => Ok(Self::Real),
            "live" => Ok(Self::Live),
            "simulated" | "sim" | "synthetic" => Err(anyhow::anyhow!(
                "data-source '{}' refused. simulated/synthetic data cannot falsify a claim about reality. either capture real data and use --data-source=real, or file the claim with --method=derived and cite the simulator's underlying claims.",
                s
            )),
            _ => Err(anyhow::anyhow!(
                "invalid --data-source '{}'. valid: real|live",
                s
            )),
        }
    }
}

impl EvidenceMethod {
    pub fn tier(&self) -> ConfidenceTier {
        match self {
            EvidenceMethod::PropTest
            | EvidenceMethod::IntegrationTest
            | EvidenceMethod::ReplayTest
            | EvidenceMethod::StatTest => ConfidenceTier::Empirical,
            EvidenceMethod::Observed => ConfidenceTier::Observed,
            EvidenceMethod::Documented => ConfidenceTier::Documented,
            EvidenceMethod::Derived => ConfidenceTier::Derived,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceMethod::PropTest => "prop-test",
            EvidenceMethod::IntegrationTest => "integration-test",
            EvidenceMethod::ReplayTest => "replay-test",
            EvidenceMethod::StatTest => "stat-test",
            EvidenceMethod::Observed => "observed",
            EvidenceMethod::Documented => "documented",
            EvidenceMethod::Derived => "derived",
        }
    }

    /// true if the method produces a re-runnable shell artifact. used by
    /// `clms rerun` to find the most recent runnable evidence on a claim.
    pub fn is_runnable(&self) -> bool {
        matches!(
            self,
            EvidenceMethod::PropTest
                | EvidenceMethod::IntegrationTest
                | EvidenceMethod::ReplayTest
                | EvidenceMethod::StatTest
        )
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "prop-test" => Ok(Self::PropTest),
            "integration-test" => Ok(Self::IntegrationTest),
            "replay-test" => Ok(Self::ReplayTest),
            "stat-test" => Ok(Self::StatTest),
            "observed" => Ok(Self::Observed),
            "documented" => Ok(Self::Documented),
            "derived" => Ok(Self::Derived),
            "unit-test" => Err(anyhow::anyhow!(
                "method 'unit-test' is not a valid evidence method. unit tests are confirmatory by construction (you pick the input and the expected output, so the test cannot disagree with you). use 'prop-test' for randomized input exploration, 'integration-test' for real external systems, 'replay-test' for frozen real-world data, or 'observed' for a captured artifact."
            )),
            "code-test" => Err(anyhow::anyhow!(
                "method 'code-test' was removed in schema 1.1. it conflated unit/integration/replay/property tests under one tier. pick one of: prop-test | integration-test | replay-test. see `clms verify --help`."
            )),
            "sim-test" => Err(anyhow::anyhow!(
                "method 'sim-test' is not a valid evidence method. running a stat-test on synthetic data only proves your simulator behaves like your simulator (circular). either capture real data and use 'stat-test --data-source=real', or file the result as 'derived' citing the simulator's underlying claims."
            )),
            _ => Err(anyhow::anyhow!(
                "invalid method '{}'. valid: prop-test|integration-test|replay-test|stat-test|observed|documented|derived",
                s
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub method: EvidenceMethod,
    pub r#ref: String,
    pub note: Option<String>,
    pub p_value: Option<f64>,
    pub sample_size: Option<u64>,
    pub test_type: Option<String>,
    pub exit_code: Option<i32>,
    pub quote: Option<String>,
    pub from_claims: Vec<u64>,
    pub ref_hash: Option<String>,
    /// shell command that produced this evidence; required for `claims rerun` to re-execute.
    pub cmd: Option<String>,
    /// blake3 hash of stdout captured at verify time. drift detection on rerun.
    pub stdout_hash: Option<String>,
    /// integration-test: required. the external system probed (url/host/endpoint).
    /// recorded so the falsification surface is auditable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// replay-test: required. path to the real-world dataset replayed.
    /// content-hashed at write-time via `ref_hash` mechanism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    /// replay-test: blake3 hash of dataset at verify time. drift detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_hash: Option<String>,
    /// stat-test: required. real | live (simulated/synthetic refused at parse time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source: Option<DataSource>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeType {
    DependsOn,
    Tests,
    Supports,
    Refines,
    Refutes,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeType::DependsOn => "depends_on",
            EdgeType::Tests => "tests",
            EdgeType::Supports => "supports",
            EdgeType::Refines => "refines",
            EdgeType::Refutes => "refutes",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub r#type: EdgeType,
    pub to_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub schema_version: String,
    pub id: Ulid,
    pub seq: u64,
    pub text: String,
    pub state: State,
    pub tags: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub edges: Vec<Edge>,
    pub agent: Option<String>,
    pub session: Option<String>,
    pub git_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub content_hash: Option<String>,
    /// archaeology v2 backfill metadata. only populated for claims born from
    /// `clms archaeology commit`. additive, non-breaking — existing claim files
    /// without this field deserialize to None and re-serialize without the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archaeology_meta: Option<ArchaeologyMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchaeologyMeta {
    pub candidate_id: String,
    pub kind: String,
    pub stake_signal: StakeSignal,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_evidence: Vec<SuggestedEvidence>,
    pub debate_transcript_ref: String,
    pub judge_rationale: String,
    pub judge_rank: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeSignal {
    pub r#where: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedEvidence {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    pub note: String,
}

impl Claim {
    pub fn confidence(&self) -> Option<ConfidenceTier> {
        self.evidence.iter().map(|e| e.method.tier()).max()
    }
}
