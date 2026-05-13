//! command-line surface: clap structs + help-text constants. nothing here
//! has runtime behavior — every `cmd_*` fn lives under `commands/` and
//! consumes one of these arg structs. fields are `pub(crate)` so the
//! command modules can read them directly.

use crate::models::{BenchmarkMetric, Estimator, HypothesisTest};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub(crate) const TOP_HELP: &str = include_str!("help_text/top.txt");
pub(crate) const ADD_HELP: &str = include_str!("help_text/add.txt");
pub(crate) const VERIFY_HELP: &str = include_str!("help_text/verify.txt");
pub(crate) const REFUTE_HELP: &str = include_str!("help_text/refute.txt");
pub(crate) const RERUN_HELP: &str = include_str!("help_text/rerun.txt");
pub(crate) const DIFF_HELP: &str = include_str!("help_text/diff.txt");
pub(crate) const MIGRATE_INTEGRITY_HELP: &str = include_str!("help_text/migrate_integrity.txt");
pub(crate) const STATS_HELP: &str = include_str!("help_text/stats.txt");
pub(crate) const ARCHAEOLOGY_HELP: &str = include_str!("help_text/archaeology.txt");
pub(crate) const SCHEMA_HELP: &str = include_str!("help_text/schema.txt");

#[derive(Parser)]
#[command(
    name = "clms",
    version,
    about = "append-only ledger of falsifiable claims, agent-optimized",
    after_help = TOP_HELP,
)]
pub(crate) struct Cli {
    #[arg(long, default_value = "default", env = "CLAIMS_FORMAT")]
    pub(crate) format: String,

    #[arg(long, env = "CLAIMS_DIR")]
    pub(crate) dir: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// log a new claim. exits non-zero if you forgot anything required.
    #[command(after_help = ADD_HELP)]
    Add(AddArgs),
    /// attach evidence to a claim and mark it verified. method-specific fields are required, no soft warnings.
    #[command(after_help = VERIFY_HELP)]
    Verify(VerifyArgs),
    /// mark a claim refuted. requires the replacing claim id.
    #[command(after_help = REFUTE_HELP)]
    Refute(RefuteArgs),
    /// inspect a single claim.
    Show { id: String },
    /// timeline of all claims (sorted by seq). refuted dimmed.
    Timeline {
        #[arg(long)]
        tag: Option<String>,
        /// hide claims stamped with these agent values (repeatable). useful for filtering archaeology backfill.
        #[arg(long = "exclude-agent")]
        exclude_agent: Vec<String>,
    },
    /// compact verified-only digest for stuffing into agent context.
    Context {
        #[arg(long)]
        tag: Option<String>,
        /// hide claims stamped with these agent values (repeatable). useful for filtering archaeology backfill.
        #[arg(long = "exclude-agent")]
        exclude_agent: Vec<String>,
    },
    /// list all currently-suspect claims (auto-flagged by cascading refutes).
    Suspect {
        /// hide claims stamped with these agent values (repeatable).
        #[arg(long = "exclude-agent")]
        exclude_agent: Vec<String>,
    },
    /// rebuild the sqlite index from the .claims/*.json source of truth.
    Reindex,
    /// upgrade legacy hash-only claim files by verifying content_hash once and writing integrity_mac.
    #[command(after_help = MIGRATE_INTEGRITY_HELP)]
    MigrateIntegrity,
    /// re-execute the stored cmd on a claim's latest runnable evidence and append fresh evidence.
    #[command(after_help = RERUN_HELP)]
    Rerun(RerunArgs),
    /// show the chronological evolution of evidence on a claim.
    #[command(after_help = DIFF_HELP)]
    DiffEvidence { id: String },
    /// ledger composition snapshot: counts per state, per tier, per method, plus the observed/empirical ratio. for xagent-style policy validators.
    #[command(after_help = STATS_HELP)]
    Stats {
        /// only count claims carrying this tag.
        #[arg(long)]
        tag: Option<String>,
        /// hide claims stamped with these agent values (repeatable).
        #[arg(long = "exclude-agent")]
        exclude_agent: Vec<String>,
    },
    /// dump top-level help + every subcommand's help in a single output. for agent context-stuffing.
    HelpAll,
    /// dump machine-readable schema (commands, evidence-method requirements, enums). for agent self-discovery.
    /// pass `methods` as the target to get just the method-descriptor table as json (agent introspection surface).
    #[command(after_help = SCHEMA_HELP)]
    Schema {
        /// optional sub-target. valid: `methods` (descriptor table only).
        target: Option<String>,
    },
    /// archaeology v2: harvest stake-signal candidates, debate via pi-subagents, commit survivors. (v1 removed.)
    #[command(after_help = ARCHAEOLOGY_HELP, subcommand_required = true, arg_required_else_help = true)]
    Archaeology {
        #[command(subcommand)]
        sub: ArchaeologySub,
    },
    /// install the bundled clms.judge / clms.proposer pi-subagents to user scope. idempotent.
    InstallAgents {
        /// overwrite existing files even if present
        #[arg(long)]
        force: bool,
        /// dry-run: print what would be written without touching disk
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ArchaeologySub {
    /// harvest candidate claims from repo signals. emits candidates.json. NEVER writes claims.
    Suggest(SuggestArgs),
    /// ingest curated survivors.json. writes pending claims with archaeology_meta. requires debate.
    Commit(CommitArgs),
    /// remove archaeology claims by session (and optional agent). idempotent cleanup.
    Purge(PurgeArgs),
}

#[derive(Args)]
pub(crate) struct SuggestArgs {
    /// hard cap on candidates (default 10, ceiling 50)
    #[arg(long, default_value_t = 10)]
    pub(crate) max: usize,
    /// signal kinds to enable. v2.0 ships only `clms-claim-annotation`. (repeatable)
    #[arg(long)]
    pub(crate) source: Vec<String>,
    /// write candidates.json to this path (default: stdout)
    #[arg(long, short = 'o')]
    pub(crate) output: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct CommitArgs {
    /// path to survivors.json from the debate phase
    #[arg(long = "from-plan")]
    pub(crate) from_plan: PathBuf,
    /// cap on committed claims (default 8)
    #[arg(long, default_value_t = 8)]
    pub(crate) keep: usize,
}

#[derive(Args)]
pub(crate) struct PurgeArgs {
    /// session stamp to match (e.g. backfill-2026-05-06T22:50:14Z)
    #[arg(long)]
    pub(crate) session: String,
    /// also restrict to this agent stamp (default: archaeology)
    #[arg(long)]
    pub(crate) agent: Option<String>,
}

#[derive(Args)]
pub(crate) struct AddArgs {
    pub(crate) text: String,
    #[arg(long)]
    pub(crate) tag: Vec<String>,
    /// claims this one is built on. if any of these are refuted later, this one is auto-flagged suspect.
    #[arg(long = "depends-on")]
    pub(crate) depends_on: Vec<u64>,
    /// claim id this one is testing (neutral edge, outcome decides if it supports/refutes).
    #[arg(long)]
    pub(crate) tests: Vec<u64>,
    /// mark this claim explicitly as unverifiable (subjective, future-dependent, etc).
    #[arg(long)]
    pub(crate) unverifiable: bool,
    #[arg(long, env = "CLAIMS_AGENT")]
    pub(crate) agent: Option<String>,
    #[arg(long, env = "CLAIMS_SESSION")]
    pub(crate) session: Option<String>,
    /// override the auto-stamped git sha. for backfill / archaeology workflows.
    #[arg(long = "git-sha")]
    pub(crate) git_sha_override: Option<String>,
    /// override the auto-stamped created_at timestamp (RFC3339). for backfill / archaeology workflows.
    #[arg(long = "created-at")]
    pub(crate) created_at_override: Option<String>,
    /// opt-in evidence-tier floor. when set, `clms verify` refuses to mark
    /// this claim Verified using evidence below the floor. valid values:
    /// empirical | observed | documented | derived. orchestrator stamps this
    /// on science claims so a downstream agent cannot satisfy with observed.
    #[arg(long = "min-tier")]
    pub(crate) min_tier: Option<String>,
}

#[derive(Args)]
pub(crate) struct VerifyArgs {
    /// seq number or ulid
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) method: String,
    #[arg(long = "ref")]
    pub(crate) r#ref: String,
    #[arg(long)]
    pub(crate) note: Option<String>,
    /// stat-test only: optional echo-check for the p_value parsed from the
    /// local JSON artifact at --ref. if supplied, it must match the artifact.
    #[arg(long)]
    pub(crate) p_value: Option<f64>,
    /// stat-test / benchmark / estimate: optional echo-check for the
    /// sample_size parsed from the local JSON artifact at --ref. if supplied,
    /// it must match the artifact.
    #[arg(long)]
    pub(crate) sample_size: Option<u64>,
    /// stat-test only: optional echo-check for the test_type parsed from the
    /// local JSON artifact at --ref. closed enum (chi-squared | t-test-paired |
    /// t-test-unpaired | t-test-one-sample | welch-t | anova |
    /// kolmogorov-smirnov | mann-whitney-u | wilcoxon-signed-rank |
    /// shapiro-wilk | anderson-darling | fisher | permutation |
    /// likelihood-ratio | chi-squared-goodness-of-fit). if supplied, it must
    /// match the artifact exactly.
    #[arg(long, value_enum)]
    pub(crate) test_type: Option<HypothesisTest>,
    #[arg(long)]
    pub(crate) exit_code: Option<i32>,
    #[arg(long)]
    pub(crate) quote: Option<String>,
    #[arg(long = "from")]
    pub(crate) from: Vec<u64>,
    /// shell command that produced this evidence. enables `claims rerun <id>` later.
    /// for stat-test / benchmark / estimate, the command must exit 0 and
    /// produce the local JSON artifact at --ref; clms parses measured values
    /// from that artifact instead of trusting cli flags.
    #[arg(long)]
    pub(crate) cmd: Option<String>,
    /// integration-test only: real external system probed (url/host/endpoint).
    /// the falsification surface must be auditable.
    #[arg(long)]
    pub(crate) target: Option<String>,
    /// replay-test only: path to real-world captured dataset (parquet/csv/jsonl).
    /// content-hashed at write time. synthetic data is not allowed.
    #[arg(long)]
    pub(crate) dataset: Option<String>,
    /// stat-test / benchmark / estimate: optional echo-check for the
    /// data_source parsed from the local JSON artifact at --ref. valid:
    /// real | live. if supplied, it must match the artifact.
    #[arg(long)]
    pub(crate) data_source: Option<String>,
    /// required when this --ref's content has changed since a prior evidence entry on the same claim.
    #[arg(long)]
    pub(crate) acknowledge_drift: bool,
    /// integration-test only: allow loopback / private-network --target.
    /// without this flag, localhost, 127/8, 0/8, ::1, and RFC1918 ranges are
    /// refused as targets because the falsifiability rationale of
    /// integration-test is "hit a real external system the author does not
    /// control" — a service on your own machine fails that test.
    #[arg(long)]
    pub(crate) allow_local: bool,
    /// benchmark only: optional echo-check for the metric parsed from the
    /// local JSON artifact at --ref.
    #[arg(long, value_enum)]
    pub(crate) metric: Option<BenchmarkMetric>,
    /// benchmark only: optional echo-check for the measured value parsed from
    /// the local JSON artifact at --ref. compared against --threshold per
    /// direction (higher- or lower-better, fixed per metric).
    #[arg(long)]
    pub(crate) metric_value: Option<f64>,
    /// benchmark only: the pass threshold. for higher-better metrics
    /// (auc, f1, accuracy, r2) metric_value >= threshold passes. for
    /// lower-better (rmse, mae, log-loss) metric_value <= threshold passes.
    #[arg(long)]
    pub(crate) threshold: Option<f64>,
    /// estimate only: optional echo-check for the estimator parsed from the
    /// local JSON artifact at --ref.
    #[arg(long, value_enum)]
    pub(crate) estimator: Option<Estimator>,
    /// estimate only: optional echo-check for the point estimate parsed from
    /// the local JSON artifact at --ref. must lie within [ci_lower, ci_upper].
    #[arg(long)]
    pub(crate) point_value: Option<f64>,
    /// estimate only: optional echo-check for the CI lower bound parsed from
    /// the local JSON artifact at --ref.
    #[arg(long)]
    pub(crate) ci_lower: Option<f64>,
    /// estimate only: optional echo-check for the CI upper bound parsed from
    /// the local JSON artifact at --ref.
    #[arg(long)]
    pub(crate) ci_upper: Option<f64>,
    /// estimate only: optional echo-check for the confidence level parsed from
    /// the local JSON artifact at --ref.
    #[arg(long)]
    pub(crate) confidence_level: Option<f64>,
}

#[derive(Args)]
pub(crate) struct RerunArgs {
    pub(crate) id: String,
    /// also acknowledge drift if the original test file's content changed.
    #[arg(long)]
    pub(crate) acknowledge_drift: bool,
}

#[derive(Args)]
pub(crate) struct RefuteArgs {
    pub(crate) id: String,
    #[arg(long)]
    pub(crate) by: u64,
    #[arg(long)]
    pub(crate) reason: String,
    /// also flag every transitive dependent as suspect.
    #[arg(long)]
    pub(crate) cascade: bool,
}
