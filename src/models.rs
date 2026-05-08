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

/// canonical descriptor for an evidence method. **single source of truth.**
///
/// the `METHODS` table below drives:
///   - `EvidenceMethod::parse` / `as_str` / `tier` / `is_runnable`
///   - `validate_evidence` dispatch (per-method validators in store.rs)
///   - cross-flag rejection in `build_evidence` (via `exclusive_flag`)
///   - `clms schema methods` json output (agent introspection)
///   - schema_value() methods array
///
/// to add an 8th method: append one row here + add one validator fn in
/// store.rs. that's it. no other touchpoints.
#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    pub name: &'static str,
    pub variant: EvidenceMethod,
    pub tier: ConfidenceTier,
    pub runnable: bool,
    pub falsification_surface: &'static str,
    pub required_fields: &'static [&'static str],
    /// the cli flag (without `--` prefix) that is exclusively valid with this
    /// method. used by build_evidence to reject `--target` on non-integration,
    /// `--dataset` on non-replay, `--data-source` on non-stat invocations.
    pub exclusive_flag: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct RefusedMethodSpec {
    pub name: &'static str,
    pub error_msg: &'static str,
}

pub const METHODS: &[MethodSpec] = &[
    MethodSpec {
        name: "prop-test",
        variant: EvidenceMethod::PropTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "randomized input generator",
        required_fields: &["ref", "exit_code", "cmd"],
        exclusive_flag: None,
    },
    MethodSpec {
        name: "integration-test",
        variant: EvidenceMethod::IntegrationTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "real external system the author does not control",
        required_fields: &["ref", "exit_code", "target", "cmd"],
        exclusive_flag: Some("target"),
    },
    MethodSpec {
        name: "replay-test",
        variant: EvidenceMethod::ReplayTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "frozen real-world dataset (synthetic refused)",
        required_fields: &["ref", "exit_code", "dataset", "dataset_hash", "cmd"],
        exclusive_flag: Some("dataset"),
    },
    MethodSpec {
        name: "stat-test",
        variant: EvidenceMethod::StatTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "real or live samples (simulated refused)",
        required_fields: &["ref", "p_value", "sample_size", "test_type", "data_source"],
        exclusive_flag: Some("data-source"),
    },
    MethodSpec {
        name: "observed",
        variant: EvidenceMethod::Observed,
        tier: ConfidenceTier::Observed,
        runnable: false,
        falsification_surface: "captured runtime artifact",
        required_fields: &["ref"],
        exclusive_flag: None,
    },
    MethodSpec {
        name: "documented",
        variant: EvidenceMethod::Documented,
        tier: ConfidenceTier::Documented,
        runnable: false,
        falsification_surface: "primary-source quote",
        required_fields: &["ref", "quote"],
        exclusive_flag: None,
    },
    MethodSpec {
        name: "derived",
        variant: EvidenceMethod::Derived,
        tier: ConfidenceTier::Derived,
        runnable: false,
        falsification_surface: "upstream claims (cascade on refute)",
        required_fields: &["from"],
        exclusive_flag: None,
    },
];

pub const REFUSED_METHODS: &[RefusedMethodSpec] = &[
    RefusedMethodSpec {
        name: "unit-test",
        error_msg: "method 'unit-test' is not a valid evidence method. unit tests are confirmatory by construction (you pick the input and the expected output, so the test cannot disagree with you). use 'prop-test' for randomized input exploration, 'integration-test' for real external systems, 'replay-test' for frozen real-world data, or 'observed' for a captured artifact.",
    },
    RefusedMethodSpec {
        name: "code-test",
        error_msg: "method 'code-test' was removed in schema 1.1. it conflated unit/integration/replay/property tests under one tier. pick one of: prop-test | integration-test | replay-test. see `clms verify --help`.",
    },
    RefusedMethodSpec {
        name: "sim-test",
        error_msg: "method 'sim-test' is not a valid evidence method. running a stat-test on synthetic data only proves your simulator behaves like your simulator (circular). either capture real data and use 'stat-test --data-source=real', or file the result as 'derived' citing the simulator's underlying claims.",
    },
];

impl EvidenceMethod {
    /// canonical descriptor for this variant.
    pub fn spec(&self) -> &'static MethodSpec {
        METHODS
            .iter()
            .find(|m| m.variant == *self)
            .expect("every EvidenceMethod variant has a MethodSpec entry")
    }

    pub fn tier(&self) -> ConfidenceTier {
        self.spec().tier
    }

    pub fn as_str(&self) -> &'static str {
        self.spec().name
    }

    /// true if the method produces a re-runnable shell artifact. used by
    /// `clms rerun` to find the most recent runnable evidence on a claim.
    pub fn is_runnable(&self) -> bool {
        self.spec().runnable
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        if let Some(spec) = METHODS.iter().find(|m| m.name == s) {
            return Ok(spec.variant);
        }
        if let Some(refused) = REFUSED_METHODS.iter().find(|r| r.name == s) {
            return Err(anyhow::anyhow!("{}", refused.error_msg));
        }
        let valid: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
        Err(anyhow::anyhow!(
            "invalid method '{}'. valid: {}",
            s,
            valid.join("|")
        ))
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
