//! The METHODS table — single source of truth for evidence-method metadata
//! — plus REFUSED_METHODS (intentionally-disallowed names + their pedagogy
//! error messages) and the EvidenceMethod impl block that reads from these
//! tables. Split out of models.rs because this is reference data, not a
//! schema declaration.
//!
//! Adding a new evidence method:
//!   1. add a variant to `pub enum EvidenceMethod` in models/enums.rs.
//!   2. append a `MethodSpec { ... }` row to METHODS below.
//!   3. add a corresponding `fn validate_<name>` in store.rs.
//! That's the entire surface. The METHODS table drives parse, as_str, tier,
//! is_runnable, validate_evidence dispatch, exclusive-flag rejection, the
//! `clms schema methods` dump, and the schema_value() methods array.

use super::enums::{ConfidenceTier, EvidenceMethod};

/// canonical descriptor for an evidence method.
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
        required_fields: &["ref", "cmd"],
        exclusive_flag: None,
    },
    MethodSpec {
        name: "integration-test",
        variant: EvidenceMethod::IntegrationTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "real external system the author does not control",
        required_fields: &["ref", "target", "cmd"],
        exclusive_flag: Some("target"),
    },
    MethodSpec {
        name: "replay-test",
        variant: EvidenceMethod::ReplayTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "frozen real-world dataset (synthetic refused)",
        required_fields: &["ref", "dataset", "dataset_hash", "cmd"],
        exclusive_flag: Some("dataset"),
    },
    MethodSpec {
        name: "stat-test",
        variant: EvidenceMethod::StatTest,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "real or live samples (simulated refused)",
        required_fields: &[
            "ref(local-json-artifact)", "cmd",
            "artifact.test_type", "artifact.p_value", "artifact.sample_size", "artifact.data_source",
        ],
        // data-source is shared with benchmark in 1.2; exclusive_flag's
        // single-owner mechanism cannot express that, so the data-source
        // gate is special-cased in check_exclusive_flags instead.
        exclusive_flag: None,
    },
    MethodSpec {
        name: "benchmark",
        variant: EvidenceMethod::Benchmark,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "measured metric on held-out real data vs declared threshold",
        required_fields: &[
            "ref(local-json-artifact)", "cmd", "threshold",
            "artifact.metric", "artifact.metric_value", "artifact.sample_size", "artifact.data_source",
        ],
        exclusive_flag: Some("metric"),
    },
    MethodSpec {
        name: "estimate",
        variant: EvidenceMethod::Estimate,
        tier: ConfidenceTier::Empirical,
        runnable: true,
        falsification_surface: "claimed point falls outside computed CI on replication",
        required_fields: &[
            "ref(local-json-artifact)", "cmd",
            "artifact.estimator", "artifact.point_value", "artifact.ci_lower", "artifact.ci_upper",
            "artifact.confidence_level", "artifact.sample_size", "artifact.data_source",
        ],
        exclusive_flag: Some("estimator"),
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
