//! All fieldless enums for the claims schema.
//!
//! Each enum gets `as_str` + (often) `all_names` + `parse` via the
//! `enum_strings!` macro defined in `crate::macros`. EvidenceMethod is
//! special: its impls live in `models/methods.rs` because they read from
//! the `METHODS` table (single source of truth for method metadata) rather
//! than a per-variant match.
//!
//! Bespoke methods stay attached to their enums here in separate impl
//! blocks: ConfidenceTier::is_at_least (tier ordering), BenchmarkMetric::
//! is_higher_better (metric direction), DataSource::parse (the
//! falsifiability-pedagogy rejection of synthetic data).
//!
//! Adding a variant: append a row to the macro call. All four downstream
//! methods (as_str/all_names/parse and any consumer iterating all_names)
//! update automatically.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Pending,
    Verified,
    Refuted,
    Unverifiable,
    Suspect,
}

enum_strings!(State, {
    Pending => "pending",
    Verified => "verified",
    Refuted => "refuted",
    Unverifiable => "unverifiable",
    Suspect => "suspect",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfidenceTier {
    Derived = 1,
    Documented = 2,
    Observed = 3,
    Empirical = 4,
}

// note: ConfidenceTier::parse accepts kebab-case as written by agents on
// `clms add --min-tier <name>`. unknown names produce a hard error listing
// the valid set so an LLM cannot bluff its way through.
enum_strings!(ConfidenceTier, err: "invalid tier '{}'. valid: {}", {
    Empirical => "empirical",
    Observed => "observed",
    Documented => "documented",
    Derived => "derived",
});

impl ConfidenceTier {
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

// impl EvidenceMethod lives in models/methods.rs (uses METHODS table).

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

enum_strings!(BenchmarkMetric, err: "invalid benchmark metric '{}' in artifact. valid: {}", {
    AucRoc => "auc-roc",
    AucPr => "auc-pr",
    F1 => "f1",
    Precision => "precision",
    Recall => "recall",
    Accuracy => "accuracy",
    BalancedAccuracy => "balanced-accuracy",
    Mcc => "mcc",
    KappaCohen => "kappa-cohen",
    R2 => "r2",
    LogLoss => "log-loss",
    Brier => "brier",
    Rmse => "rmse",
    Mae => "mae",
    Mape => "mape",
});

impl BenchmarkMetric {
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

enum_strings!(Estimator, err: "invalid estimator '{}' in artifact. valid: {}", {
    Mean => "mean",
    Median => "median",
    GeometricMean => "geometric-mean",
    StdDev => "std-dev",
    StdError => "std-error",
    Variance => "variance",
    Skewness => "skewness",
    Kurtosis => "kurtosis",
    CohensD => "cohens-d",
    OddsRatio => "odds-ratio",
    RiskRatio => "risk-ratio",
    Correlation => "correlation",
    SpearmanRho => "spearman-rho",
});

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

// DataSource has a bespoke parse (rejects 'simulated'/'sim'/'synthetic' with
// a specific falsifiability-pedagogy message and uses 'real|live' without
// spaces around the pipe in the catch-all error), so only as_str is generated.
enum_strings!(DataSource, {
    Real => "real",
    Live => "live",
});

impl DataSource {
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

// HypothesisTest::all_names is used by the `clms schema` dump and the error
// path when an unknown string sneaks in via a legacy claim file.
enum_strings!(HypothesisTest, err: "invalid hypothesis test '{}' in artifact. valid: {}", {
    ChiSquared => "chi-squared",
    ChiSquaredGoodnessOfFit => "chi-squared-goodness-of-fit",
    TTestPaired => "t-test-paired",
    TTestUnpaired => "t-test-unpaired",
    TTestOneSample => "t-test-one-sample",
    WelchT => "welch-t",
    Anova => "anova",
    KolmogorovSmirnov => "kolmogorov-smirnov",
    MannWhitneyU => "mann-whitney-u",
    WilcoxonSignedRank => "wilcoxon-signed-rank",
    ShapiroWilk => "shapiro-wilk",
    AndersonDarling => "anderson-darling",
    Fisher => "fisher",
    Permutation => "permutation",
    LikelihoodRatio => "likelihood-ratio",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeType {
    DependsOn,
    Tests,
    Supports,
    Refines,
    Refutes,
}

// EdgeType uses snake_case (depends_on) not kebab-case; the macro doesn't
// enforce a naming scheme since each variant has its name written explicitly.
enum_strings!(EdgeType, {
    DependsOn => "depends_on",
    Tests => "tests",
    Supports => "supports",
    Refines => "refines",
    Refutes => "refutes",
});
