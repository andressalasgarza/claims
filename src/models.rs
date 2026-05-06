use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub const SCHEMA_VERSION: &str = "1.0";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceMethod {
    StatTest,
    CodeTest,
    Observed,
    Documented,
    Derived,
}

impl EvidenceMethod {
    pub fn tier(&self) -> ConfidenceTier {
        match self {
            EvidenceMethod::StatTest | EvidenceMethod::CodeTest => ConfidenceTier::Empirical,
            EvidenceMethod::Observed => ConfidenceTier::Observed,
            EvidenceMethod::Documented => ConfidenceTier::Documented,
            EvidenceMethod::Derived => ConfidenceTier::Derived,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceMethod::StatTest => "stat-test",
            EvidenceMethod::CodeTest => "code-test",
            EvidenceMethod::Observed => "observed",
            EvidenceMethod::Documented => "documented",
            EvidenceMethod::Derived => "derived",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "stat-test" => Ok(Self::StatTest),
            "code-test" => Ok(Self::CodeTest),
            "observed" => Ok(Self::Observed),
            "documented" => Ok(Self::Documented),
            "derived" => Ok(Self::Derived),
            _ => Err(anyhow::anyhow!(
                "invalid method '{}'. valid: stat-test|code-test|observed|documented|derived",
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
}

impl Claim {
    pub fn confidence(&self) -> Option<ConfidenceTier> {
        self.evidence.iter().map(|e| e.method.tier()).max()
    }
}
