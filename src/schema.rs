//! schema for `clms schema` output. agent-facing introspection surface.
//!
//! WARNING: this output is consumed by agents to learn the data model.
//! changes here MUST be additive (new keys ok, removing or renaming keys
//! is a breaking change). bump `version` whenever the shape changes.

use crate::models::{BenchmarkMetric, Estimator, HypothesisTest, METHODS, REFUSED_METHODS};
use serde_json::{json, Value};

/// machine-readable dump of the method-descriptor table. agent introspection
/// surface for `clms schema methods`. driven directly from `models::METHODS`
/// + `models::REFUSED_METHODS` so it cannot drift from validation logic.
///
/// stable contract for agents:
///   { "version": "1.3",
///     "methods": [ { name, tier, runnable, falsification_surface,
///                    required_fields, exclusive_flag } ... ],
///     "refused_methods": [ { name, error_msg } ... ] }
pub fn methods_table() -> Value {
    let methods: Vec<Value> = METHODS
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "tier": m.tier.as_str(),
                "runnable": m.runnable,
                "falsification_surface": m.falsification_surface,
                "required_fields": m.required_fields,
                "exclusive_flag": m.exclusive_flag,
            })
        })
        .collect();
    let refused: Vec<Value> = REFUSED_METHODS
        .iter()
        .map(|r| json!({ "name": r.name, "error_msg": r.error_msg }))
        .collect();
    json!({
        "version": "1.3",
        "methods": methods,
        "refused_methods": refused,
    })
}

pub fn schema_value() -> Value {
    json!({
        "version": "2.2",
        "archaeology": {
            "version": "archaeology/v2",
            "contract": "candidacy engine, NOT verification engine. always writes state=pending. promotion via clms verify.",
            "agent_stamp": "archaeology",
            "session_stamp_format": "backfill-<rfc3339-ts>",
            "phases": {
                "1_harvest": "clms archaeology suggest. reads two intent surfaces: in-source `// clms-claim:` annotations AND `.archaeology/proposals.json` (agent-written, optional). bounded N (default 10, ceiling 50).",
                "2_debate": "orchestrator-agnostic. pi-subagents reference impl uses .pi/agents/clms-judge.md (clms.judge runtime). drop is default.",
                "3_commit": "clms archaeology commit --from-plan <survivors.json>. validates schema, writes state=pending claims with archaeology_meta. transcripts at .archaeology/<session>/."
            },
            "intent_surfaces": {
                "human_in_source": {
                    "location": "// clms-claim: ... or # clms-claim: ... in *.rs/.py/.ts/.go/...",
                    "persistence": "durable, version-controlled"
                },
                "agent_manifest": {
                    "location": ".archaeology/proposals.json",
                    "persistence": "ephemeral, regenerable",
                    "reference_agent": ".pi/agents/clms-proposer.md",
                    "schema": {
                        "version": "archaeology/v2",
                        "proposals": [{"text": "required", "where": "required", "snippet": "optional", "suggested_evidence": "optional []"}]
                    }
                }
            },
            "signals_v2": [
                {"kind": "clms-claim-annotation", "shipped": true, "description": "// clms-claim: in source code OR row in .archaeology/proposals.json. one signal kind, two intent surfaces."}
            ],
            "signals_deferred": [
                "test-name-invariant", "type-marked", "mb-verify-task",
                "cm-structural", "readme-assertion", "docstring-invariant"
            ],
            "protocol": {
                "candidates_schema_version": "archaeology/v2",
                "survivors_invariants": [
                    "input is candidates.json matching the schema",
                    "output is survivors.json with debate.judge per row and keep:bool per row",
                    "drop is structural default (keep:true requires affirmative debate.judge.verdict=keep)",
                    "K cap is post-hoc enforced by `clms archaeology commit`, not by the orchestrator"
                ]
            },
            "refusal_conditions": [
                "debate=null on a keep:true row",
                "keep:true with debate.judge.verdict != keep",
                "count(keep:true) > --keep cap",
                "version != 'archaeology/v2'",
                "candidate_id collision with existing claim (idempotent skip, not error)"
            ],
            "filter_live_context": "--exclude-agent archaeology on context/timeline/suspect",
            "v1_removed": "v1 git+mb auto-transcribe behavior removed. use `clms archaeology purge --session <stamp>` for cleanup."
        },
        "states": ["pending", "verified", "refuted", "unverifiable", "suspect"],
        "confidence_tiers": [
            {"name": "derived",    "rank": 1},
            {"name": "documented", "rank": 2},
            {"name": "observed",   "rank": 3},
            {"name": "empirical",  "rank": 4}
        ],
        "min_tier": {
            "description": "opt-in tier floor stamped on a claim at write time. when set, `clms verify` refuses to mark the claim verified using evidence below the floor. set via `clms add --min-tier <tier>`. valid: empirical | observed | documented | derived.",
            "semantics": "evidence.method.tier (looked up via the methods table) is compared to claim.min_tier. evidence at or above the floor is accepted; below the floor is refused with a list of allowed methods at the floor.",
            "backward_compat": "absent on claims that did not set it; behaves exactly as pre-feature when absent."
        },
        "integrity": {
            "mode": "strict-only",
            "mandatory_field": "integrity_mac",
            "legacy_migration": "run `clms migrate-integrity` once. migration verifies each existing content_hash checkpoint, refuses mismatches, then writes keyed integrity_mac values.",
            "obsolete_marker": ".claims/.integrity.strict is ignored/obsolete; marker deletion cannot downgrade integrity checks.",
            "repair_mode": "CLAIMS_REPAIR=1 is forensic read-only. all mutating commands, including migrate-integrity, refuse under repair mode."
        },
        "edge_types": ["depends_on", "tests", "supports", "refines", "refutes"],
        "output_formats": ["default", "human", "ai"],
        "exit_codes": {
            "0": "success",
            "1": "runtime error (validation, drift, not found, etc)",
            "2": "argument parse error (missing/invalid flag)"
        },
        "error_envelope_ai": {
            "description": "under --format ai, errors emit a single-line json object on stderr",
            "fields": {
                "error":     "string, human message",
                "kind":      "\"clap\" | \"runtime\"",
                "code":      "integer exit code",
                "clap_kind": "clap ErrorKind variant (clap errors only)",
                "field":     "offending --flag (clap errors only, when extractable)"
            }
        },
        "falsifiability_principle": "every method's evidence must come from a source the author does not fully control. unit-test and sim-test are intentionally absent: they are confirmatory by construction (the author picks both the input and the expected output, or generates data from the same assumptions being tested) and cannot disagree with the claim.",
        "refused_methods": {
            "unit-test": "confirmatory only. parse-time error. use prop-test for randomized inputs, integration-test for real systems, replay-test for real captured data, or observed for an artifact.",
            "code-test": "removed in schema 1.1. ambiguous about falsification surface. pick prop-test | integration-test | replay-test.",
            "sim-test": "running stats on synthetic data only proves the simulator behaves like itself (circular). use stat-test --data-source=real, or file as derived citing the simulator's underlying claims."
        },
        "evidence_methods": {
            "prop-test": {
                "required": ["ref", "exit_code", "cmd"],
                "recommended": [],
                "confidence": "empirical",
                "falsification_surface": "randomized input generator (proptest/quickcheck/fuzz). a counterexample falsifies the property.",
                "notes": "file at --ref is content-hashed for drift detection"
            },
            "integration-test": {
                "required": ["ref", "exit_code", "target", "cmd"],
                "recommended": [],
                "confidence": "empirical",
                "falsification_surface": "the real external system at --target. the test must hit a system the author does not control (api/db/host).",
                "notes": "file at --ref is content-hashed; --target is recorded for audit"
            },
            "replay-test": {
                "required": ["ref", "exit_code", "dataset", "cmd"],
                "recommended": [],
                "confidence": "empirical",
                "falsification_surface": "frozen real-world capture at --dataset. data was generated independently of the code being tested.",
                "notes": "both --ref and --dataset are content-hashed at write time. synthetic datasets violate the contract."
            },
            "stat-test": {
                "required": ["ref (local json artifact)", "cmd"],
                "artifact_fields": ["test_type", "p_value", "sample_size", "data_source"],
                "recommended": ["test_type", "p_value", "sample_size", "data_source"],
                "confidence": "empirical",
                "falsification_surface": "real or live data samples. simulated/synthetic data is rejected at parse time.",
                "data_source_values": ["real", "live"],
                "test_type_values": HypothesisTest::all_names(),
                "notes": "--cmd must produce the local JSON artifact at --ref and exit 0. clms parses p_value/sample metadata from that artifact instead of trusting cli flags; if the matching flags are supplied, they are treated as consistency checks and must match exactly."
            },
            "benchmark": {
                "required": ["ref (local json artifact)", "cmd", "threshold"],
                "artifact_fields": ["metric", "metric_value", "sample_size", "data_source"],
                "recommended": ["metric", "metric_value", "sample_size", "data_source"],
                "confidence": "empirical",
                "falsification_surface": "measured metric on held-out real data vs declared threshold. miss the threshold and clms refuses verification.",
                "data_source_values": ["real", "live"],
                "metric_values": BenchmarkMetric::all_names(),
                "notes": "each metric has fixed direction. higher-better: auc-roc, auc-pr, f1, precision, recall, accuracy, balanced-accuracy, mcc, kappa-cohen, r2. lower-better: log-loss, brier, rmse, mae, mape. --cmd must produce the local JSON artifact at --ref and exit 0. clms parses metric/sample metadata from that artifact instead of trusting cli flags; if the matching flags are supplied, they are treated as consistency checks."
            },
            "estimate": {
                "required": ["ref (local json artifact)", "cmd"],
                "artifact_fields": ["estimator", "point_value", "ci_lower", "ci_upper", "confidence_level", "sample_size", "data_source"],
                "recommended": ["estimator", "point_value", "ci_lower", "ci_upper", "confidence_level", "sample_size", "data_source"],
                "confidence": "empirical",
                "falsification_surface": "claimed point estimate falls outside its declared CI on replication. clms enforces shape: ci_lower <= point_value <= ci_upper, 0 < confidence_level < 1.",
                "data_source_values": ["real", "live"],
                "estimator_values": Estimator::all_names(),
                "notes": "point + CI describe a statistical quantity (mean, skew, cohens-d, etc.) with sampling uncertainty. --cmd must produce the local JSON artifact at --ref and exit 0. clms parses estimator/CI metadata from that artifact instead of trusting cli flags; if the matching flags are supplied, they are treated as consistency checks."
            },
            "observed": {
                "required": ["ref"],
                "recommended": [],
                "confidence": "observed",
                "falsification_surface": "a captured artifact (tx hash, log line, response). its absence or contradiction would falsify.",
                "notes": "runtime data; --ref is path/url/hash"
            },
            "documented": {
                "required": ["ref", "quote"],
                "recommended": [],
                "confidence": "documented",
                "falsification_surface": "primary-source document. quote can be missing, edited, or contradicted.",
                "notes": "--ref is the doc url; --quote is exact text"
            },
            "derived": {
                "required": ["from (>=2)"],
                "recommended": [],
                "confidence": "derived",
                "falsification_surface": "upstream claims. refutation of any cited claim cascades.",
                "notes": "inference from at least 2 other claim ids"
            }
        },
        "env_vars": {
            "CLAIMS_FORMAT":   "default output format (default|human|ai)",
            "CLAIMS_DIR":      "working directory containing .claims/",
            "CLAIMS_AGENT":    "auto-stamp every write with this agent name",
            "CLAIMS_SESSION":  "auto-stamp every write with this session id",
            "CLAIMS_BACKFILL":            "=1 authorizes --git-sha-override / --created-at-override on `clms add` (archaeology / forensic reconstruction only).",
            "CLAIMS_REPAIR":              "=1 disables claim-integrity verification in read_claim for forensic recovery. read-only: all mutating commands, including migrate-integrity, refuse under this flag.",
            "CLAIMS_INTEGRITY_KEY_FILE":  "path to the keyed integrity material used to sign/verify claim files. defaults to ~/.clms/integrity.key and is auto-created on first write.",
            "CLAIMS_INTEGRITY_KEY_HEX":   "32-byte (64 hex chars) integrity key supplied directly via env. overrides CLAIMS_INTEGRITY_KEY_FILE."
        }
    })
}
