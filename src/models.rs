//! Schema types for the claims ledger.
//!
//! Layout:
//!   models.rs        -- this file: SCHEMA_VERSION + data structs (Claim,
//!                       Evidence, Edge, ArchaeologyMeta, StakeSignal,
//!                       SuggestedEvidence) + impl Claim. Plus re-exports
//!                       of the sub-modules so existing `use crate::models::*`
//!                       sites compile without change.
//!   models/enums.rs  -- fieldless enums (State, ConfidenceTier,
//!                       EvidenceMethod, BenchmarkMetric, Estimator,
//!                       DataSource, HypothesisTest, EdgeType) + their
//!                       as_str/parse/all_names via `enum_strings!`.
//!   models/methods.rs -- METHODS table (single source of truth for evidence
//!                       method metadata), REFUSED_METHODS, and the
//!                       EvidenceMethod impl that reads from those tables.

mod enums;
mod methods;

pub use enums::*;
pub use methods::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub const SCHEMA_VERSION: &str = "1.3";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub method: EvidenceMethod,
    pub r#ref: String,
    pub note: Option<String>,
    pub p_value: Option<f64>,
    pub sample_size: Option<u64>,
    pub test_type: Option<HypothesisTest>,
    pub exit_code: Option<i32>,
    pub quote: Option<String>,
    pub from_claims: Vec<u64>,
    pub ref_hash: Option<String>,
    /// shell command that produced this evidence; required for `claims rerun` to re-execute.
    pub cmd: Option<String>,
    /// blake3 hash of `cmd` at verify time. on rerun, the stored cmd is re-hashed
    /// and compared against this; mismatch means the json was tampered and rerun
    /// MUST refuse to execute. closes the "plant cmd in tampered .claims/*.json,
    /// run rerun, get arbitrary code execution" attack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_hash: Option<String>,
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
    /// benchmark: required. classifier/regression metric being measured.
    /// direction (higher- or lower-better) is fixed per variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<BenchmarkMetric>,
    /// benchmark: required. agent-declared measured value of `metric` on the
    /// held-out evaluation. clms compares against `threshold` per direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_value: Option<f64>,
    /// benchmark: required. the pass threshold. for higher-better metrics,
    /// metric_value >= threshold passes. for lower-better, metric_value <=
    /// threshold passes. mismatch = hard refusal, no state transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// estimate: required. statistical quantity being reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<Estimator>,
    /// estimate: required. agent-declared point estimate of `estimator`.
    /// must lie within [ci_lower, ci_upper] or verification is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_value: Option<f64>,
    /// estimate: required. lower bound of the confidence interval. must be
    /// finite and <= point_value <= ci_upper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci_lower: Option<f64>,
    /// estimate: required. upper bound of the confidence interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci_upper: Option<f64>,
    /// estimate: required. confidence level, in (0, 1). typical: 0.90 / 0.95 / 0.99.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_level: Option<f64>,
    pub recorded_at: DateTime<Utc>,
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
    /// keyed message-authentication code (MAC) over the canonical claim bytes.
    /// unlike content_hash, this is not forgeable by anyone who can merely edit
    /// the json file; they also need the signing key material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_mac: Option<String>,
    /// archaeology v2 backfill metadata. only populated for claims born from
    /// `clms archaeology commit`. additive, non-breaking — existing claim files
    /// without this field deserialize to None and re-serialize without the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archaeology_meta: Option<ArchaeologyMeta>,
    /// true if the claim was created with --git-sha or --created-at overrides
    /// under CLAIMS_BACKFILL=1. lets a reviewer distinguish "this is a live
    /// claim with auto-stamped provenance" from "this provenance was
    /// retroactively asserted by an agent". additive; absent on old records.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backfilled: bool,
    /// opt-in evidence-tier floor. when set, `clms verify` refuses to
    /// transition pending -> verified using evidence whose tier is below
    /// this floor. lets an orchestrator declare "this is a science claim,
    /// demand empirical evidence" at write time and have clms enforce the
    /// intent structurally — agents cannot shop down to observed at verify
    /// time. additive; claims without min_tier behave exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tier: Option<ConfidenceTier>,
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
