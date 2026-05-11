use chrono::{DateTime, Utc};
use clap::ValueEnum;
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

    /// parse a user-facing tier name. accepts kebab-case as written by agents
    /// on `clms add --min-tier <name>`. unknown names produce a hard error
    /// listing the valid set so an LLM cannot bluff its way through.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "empirical" => Ok(ConfidenceTier::Empirical),
            "observed" => Ok(ConfidenceTier::Observed),
            "documented" => Ok(ConfidenceTier::Documented),
            "derived" => Ok(ConfidenceTier::Derived),
            other => Err(anyhow::anyhow!(
                "invalid tier '{}'. valid: empirical | observed | documented | derived",
                other
            )),
        }
    }

    /// true if `self` is at least as strong as `floor`. encapsulates the
    /// ordering direction (Empirical=4 strongest, Derived=1 weakest) so the
    /// comparison cannot silently flip if the discriminants are ever renumbered.
    pub fn is_at_least(self, floor: ConfidenceTier) -> bool {
        (self as u8) >= (floor as u8)
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
    /// classifier/regression metric measured against a declared threshold on
    /// held-out real data. distinct from stat-test (no p-value, has threshold +
    /// direction). requires --ref --cmd --metric --metric-value --threshold
    /// --sample-size --data-source.
    Benchmark,
    /// point estimate with confidence interval on real data. distinct from
    /// benchmark (CI shape, not threshold). requires --ref --cmd --estimator
    /// --point-value --ci-lower --ci-upper --confidence-level --sample-size
    /// --data-source.
    Estimate,
    /// captured runtime artifact (tx hash, log line, response). requires --ref.
    Observed,
    /// primary-source documentation + exact quote. requires --ref --quote.
    Documented,
    /// inference from at least two other claims. requires --from N (>=2).
    Derived,
}

/// metrics accepted on `benchmark --metric`. each has a fixed direction:
/// `is_higher_better()` tells the validator whether metric_value must be
/// >= or <= threshold for verification to succeed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum BenchmarkMetric {
    AucRoc, AucPr, F1, Precision, Recall, Accuracy, BalancedAccuracy, Mcc,
    KappaCohen, R2,
    LogLoss, Brier, Rmse, Mae, Mape,
}

impl BenchmarkMetric {
    pub fn as_str(&self) -> &'static str {
        match self {
            BenchmarkMetric::AucRoc => "auc-roc",
            BenchmarkMetric::AucPr => "auc-pr",
            BenchmarkMetric::F1 => "f1",
            BenchmarkMetric::Precision => "precision",
            BenchmarkMetric::Recall => "recall",
            BenchmarkMetric::Accuracy => "accuracy",
            BenchmarkMetric::BalancedAccuracy => "balanced-accuracy",
            BenchmarkMetric::Mcc => "mcc",
            BenchmarkMetric::KappaCohen => "kappa-cohen",
            BenchmarkMetric::R2 => "r2",
            BenchmarkMetric::LogLoss => "log-loss",
            BenchmarkMetric::Brier => "brier",
            BenchmarkMetric::Rmse => "rmse",
            BenchmarkMetric::Mae => "mae",
            BenchmarkMetric::Mape => "mape",
        }
    }

    /// true if higher metric_value is better (AUC, F1, accuracy, R² family).
    /// false if lower is better (loss / error metrics).
    pub fn is_higher_better(&self) -> bool {
        match self {
            BenchmarkMetric::AucRoc
            | BenchmarkMetric::AucPr
            | BenchmarkMetric::F1
            | BenchmarkMetric::Precision
            | BenchmarkMetric::Recall
            | BenchmarkMetric::Accuracy
            | BenchmarkMetric::BalancedAccuracy
            | BenchmarkMetric::Mcc
            | BenchmarkMetric::KappaCohen
            | BenchmarkMetric::R2 => true,
            BenchmarkMetric::LogLoss
            | BenchmarkMetric::Brier
            | BenchmarkMetric::Rmse
            | BenchmarkMetric::Mae
            | BenchmarkMetric::Mape => false,
        }
    }

    pub fn all_names() -> &'static [&'static str] {
        &[
            "auc-roc", "auc-pr", "f1", "precision", "recall", "accuracy",
            "balanced-accuracy", "mcc", "kappa-cohen", "r2",
            "log-loss", "brier", "rmse", "mae", "mape",
        ]
    }
}

/// estimators accepted on `estimate --estimator`. each names a statistical
/// quantity whose value is reported with a confidence interval (CI). clms
/// enforces the CI shape (ci_lower <= point_value <= ci_upper, confidence
/// in (0,1)) but is agnostic about the estimator's semantics — those live
/// in the script behind --cmd.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum Estimator {
    Mean,
    Median,
    GeometricMean,
    StdDev,
    StdError,
    Variance,
    Skewness,
    Kurtosis,
    CohensD,
    OddsRatio,
    RiskRatio,
    Correlation,
    SpearmanRho,
}

impl Estimator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Estimator::Mean => "mean",
            Estimator::Median => "median",
            Estimator::GeometricMean => "geometric-mean",
            Estimator::StdDev => "std-dev",
            Estimator::StdError => "std-error",
            Estimator::Variance => "variance",
            Estimator::Skewness => "skewness",
            Estimator::Kurtosis => "kurtosis",
            Estimator::CohensD => "cohens-d",
            Estimator::OddsRatio => "odds-ratio",
            Estimator::RiskRatio => "risk-ratio",
            Estimator::Correlation => "correlation",
            Estimator::SpearmanRho => "spearman-rho",
        }
    }

    pub fn all_names() -> &'static [&'static str] {
        &[
            "mean", "median", "geometric-mean",
            "std-dev", "std-error", "variance",
            "skewness", "kurtosis",
            "cohens-d", "odds-ratio", "risk-ratio",
            "correlation", "spearman-rho",
        ]
    }
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

/// closed set of hypothesis tests accepted on `stat-test --test-type`.
/// previously `Option<String>` with zero validation — agents could write
/// 'AUC' or 'MyCoolTest' and clms would store it. that broke the falsifiability
/// audit trail because the same string could mean different things across
/// claims, and unknown test names made reproducing the test impossible.
///
/// to add a test: append one variant. serde + clap derive use kebab-case so
/// `ChiSquaredGoodnessOfFit` becomes `chi-squared-goodness-of-fit` on the cli.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum HypothesisTest {
    /// pearson chi-squared test of independence / homogeneity.
    ChiSquared,
    /// chi-squared goodness-of-fit test against a specified distribution.
    ChiSquaredGoodnessOfFit,
    /// paired two-sample t-test (matched pairs).
    TTestPaired,
    /// unpaired (independent samples) t-test, equal variance.
    TTestUnpaired,
    /// one-sample t-test against a hypothesized mean.
    TTestOneSample,
    /// welch's t-test (unpaired, unequal variances).
    WelchT,
    /// one-way analysis of variance.
    Anova,
    /// kolmogorov-smirnov two-sample (or one-sample-vs-distribution).
    KolmogorovSmirnov,
    /// mann-whitney U / wilcoxon rank-sum (non-parametric).
    MannWhitneyU,
    /// wilcoxon signed-rank (paired, non-parametric).
    WilcoxonSignedRank,
    /// shapiro-wilk normality test.
    ShapiroWilk,
    /// anderson-darling goodness-of-fit.
    AndersonDarling,
    /// fisher's exact test (2x2 contingency).
    Fisher,
    /// permutation / randomization test.
    Permutation,
    /// likelihood-ratio test (nested models).
    LikelihoodRatio,
}

impl HypothesisTest {
    pub fn as_str(&self) -> &'static str {
        match self {
            HypothesisTest::ChiSquared => "chi-squared",
            HypothesisTest::ChiSquaredGoodnessOfFit => "chi-squared-goodness-of-fit",
            HypothesisTest::TTestPaired => "t-test-paired",
            HypothesisTest::TTestUnpaired => "t-test-unpaired",
            HypothesisTest::TTestOneSample => "t-test-one-sample",
            HypothesisTest::WelchT => "welch-t",
            HypothesisTest::Anova => "anova",
            HypothesisTest::KolmogorovSmirnov => "kolmogorov-smirnov",
            HypothesisTest::MannWhitneyU => "mann-whitney-u",
            HypothesisTest::WilcoxonSignedRank => "wilcoxon-signed-rank",
            HypothesisTest::ShapiroWilk => "shapiro-wilk",
            HypothesisTest::AndersonDarling => "anderson-darling",
            HypothesisTest::Fisher => "fisher",
            HypothesisTest::Permutation => "permutation",
            HypothesisTest::LikelihoodRatio => "likelihood-ratio",
        }
    }

    /// every known canonical name. used by the `clms schema` dump and the
    /// error path when an unknown string sneaks in via a legacy claim file.
    pub fn all_names() -> &'static [&'static str] {
        &[
            "chi-squared",
            "chi-squared-goodness-of-fit",
            "t-test-paired",
            "t-test-unpaired",
            "t-test-one-sample",
            "welch-t",
            "anova",
            "kolmogorov-smirnov",
            "mann-whitney-u",
            "wilcoxon-signed-rank",
            "shapiro-wilk",
            "anderson-darling",
            "fisher",
            "permutation",
            "likelihood-ratio",
        ]
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
            "ref", "cmd", "metric", "metric_value", "threshold", "sample_size", "data_source",
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
            "ref", "cmd", "estimator", "point_value", "ci_lower", "ci_upper",
            "confidence_level", "sample_size", "data_source",
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
