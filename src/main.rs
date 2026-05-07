mod archaeology;
mod git;
mod models;
mod output;
mod store;

use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use colored::*;
use models::{Claim, Edge, EdgeType, Evidence, EvidenceMethod, State, SCHEMA_VERSION};
use output::OutputFormat;
use std::path::PathBuf;
use store::{
    cascade_suspect, detect_drift, hash_ref_if_local, latest_runnable_evidence, run_cmd,
    validate_evidence, Store,
};
use ulid::Ulid;

const TOP_HELP: &str = r#"
QUICKSTART
  clms add "polymarket lags binance ~300ms" --tag market:btc
  clms verify 1 --method stat-test --ref ./test.py --cmd "python3 ./test.py" \
               --p-value 0.003 --sample-size 4821 --test-type ks
  clms add "we can arb this lag" --depends-on 1
  clms refute 1 --by 3 --reason "lookahead bias" --cascade
  clms timeline
  clms context --format ai             # for stuffing into agent context

STATES
  pending      logged, no evidence yet
  verified     evidence attached, passed validation
  refuted      proven wrong, points to replacement claim
  unverifiable explicitly cannot be tested (subjective, future-dependent)
  suspect      a claim it depended on was refuted; needs re-verification

CONFIDENCE TIERS (auto-derived from evidence method)
  empirical    stat-test or code-test    (strongest)
  observed     observed (runtime data)
  documented   official primary source
  derived      inference from >= 2 other claims

FOR AGENTS
  always run with --format ai for json output. set CLAIMS_AGENT and
  CLAIMS_SESSION env vars so every write is auto-stamped. before adding a
  claim, run `clms context --format ai` to load known truth, then list
  every existing claim that must hold for yours via --depends-on.

  agent self-discovery (run once, cache the result):
    clms --format ai schema     # full requirement matrix + enums + envelopes
    clms help-all               # every subcommand's long help in one shot

  under --format ai, errors emit a single-line json envelope on stderr:
    {"error":"...","kind":"clap|runtime","code":1|2,...}
  exit 2 = bad args (clap), exit 1 = runtime (validation, drift, not found).

  archaeology v2 (candidacy engine, never verifies):
    clms archaeology suggest -o candidates.json     # phase 1: harvest
    # then debate via pi-subagents .pi/agents/clms-judge.md → survivors.json
    clms archaeology commit --from-plan survivors.json   # phase 3: pending
    clms context --exclude-agent archaeology             # filter from ctx
"#;

#[derive(Parser)]
#[command(
    name = "clms",
    version,
    about = "append-only ledger of falsifiable claims, agent-optimized",
    after_help = TOP_HELP,
)]
struct Cli {
    #[arg(long, default_value = "default", env = "CLAIMS_FORMAT")]
    format: String,

    #[arg(long, env = "CLAIMS_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

const ADD_HELP: &str = r#"
EXAMPLES
  clms add "polymarket api rate limit is 60/min" --tag infra
  clms add "we can arb this lag" --depends-on 1 --depends-on 3 --tag strategy
  clms add "mm bots withdraw at >50k size" --unverifiable --tag market

NOTES
  - --depends-on registers prerequisite claims. if any is later refuted with
    --cascade, this claim auto-flips to suspect.
  - --unverifiable is for claims you genuinely cannot test (subjective,
    future-dependent). honesty bucket. otherwise the claim starts pending.
  - claim text should be falsifiable. "x lags y by ~300ms" not "latency is bad".
"#;

const VERIFY_HELP: &str = r#"
EVIDENCE METHODS (each has REQUIRED fields, no soft warnings, exit 1 if missing)
  stat-test   --ref --test-type --p-value --sample-size  [--cmd recommended]
  code-test   --ref --exit-code                          [--cmd recommended]
  observed    --ref
  documented  --ref --quote "<exact text from doc>"
  derived     --from <id> --from <id>          (min 2)

EXAMPLES
  clms verify 1 --method stat-test --ref ./test.py \
    --test-type ks --p-value 0.003 --sample-size 4821 \
    --cmd "python3 ./test.py"

  clms verify 2 --method code-test --ref ./bench.sh --exit-code 0 \
    --cmd "bash ./bench.sh"

  clms verify 4 --method documented --ref https://docs.poly.com/api/limits \
    --quote "default rate limit is 100 requests per minute"

DRIFT
  if --ref's file content has changed since a prior evidence entry on this
  same claim, verify will refuse with exit 1 and show both hashes. add
  --acknowledge-drift if the iteration is intentional. if the new result
  contradicts the prior conclusion, you should refute the claim and write a
  new one instead of acknowledging drift.

CONFIDENCE
  auto-derived from method. cannot be set manually. add stronger evidence
  later (e.g. stat-test on top of documented) to bump confidence tier.
"#;

const REFUTE_HELP: &str = r#"
EXAMPLES
  clms refute 1 --by 7 --reason "lookahead bias in original test"
  clms refute 1 --by 7 --reason "..." --cascade   # auto-suspect dependents

NOTES
  - <id> = claim being refuted. --by <id> = the claim that supersedes it.
  - the replacement claim must already exist. typical workflow: write the
    new claim first with `clms add ...`, verify it, THEN refute the old one.
  - --cascade walks the depends_on graph and flags every transitive
    dependent of <id> as suspect. without --cascade you get a warning
    listing them but no auto-flag. use --cascade unless you have a reason
    not to.
"#;

const RERUN_HELP: &str = r#"
EXAMPLES
  clms rerun 1                       # re-execute stored cmd, append evidence
  clms rerun 1 --acknowledge-drift   # acknowledge if test file changed

NOTES
  - requires the original verify to have stored a --cmd. if missing, errors.
  - finds the latest evidence on the claim with method in {stat-test, code-test}
    and a stored cmd, re-executes via `sh -c "<cmd>"`, captures exit code +
    stdout hash + new ref_hash, appends as fresh evidence.
  - if the test file's content changed since prior evidence, drift is
    detected and rerun refuses with exit 1 unless --acknowledge-drift is set.
  - rerun does NOT change the claim's state. use it to confirm a verified
    claim still holds, or to capture fresh evidence over time.
"#;

const DIFF_HELP: &str = r#"
EXAMPLES
  clms diff-evidence 1
  clms --format ai diff-evidence 1   # full json for agent inspection

NOTES
  shows every evidence entry on a claim chronologically with hashes,
  p-values, exit codes, and notes. useful for spotting silent drift
  (file hashes that changed between runs) and tracking how a claim's
  support has evolved over multiple verify/rerun cycles.
"#;

#[derive(Subcommand)]
enum Cmd {
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
    /// re-execute the stored cmd on a claim's latest runnable evidence and append fresh evidence.
    #[command(after_help = RERUN_HELP)]
    Rerun(RerunArgs),
    /// show the chronological evolution of evidence on a claim.
    #[command(after_help = DIFF_HELP)]
    DiffEvidence { id: String },
    /// dump top-level help + every subcommand's help in a single output. for agent context-stuffing.
    HelpAll,
    /// dump machine-readable schema (commands, evidence-method requirements, enums). for agent self-discovery.
    #[command(after_help = SCHEMA_HELP)]
    Schema,
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

#[derive(clap::Subcommand)]
enum ArchaeologySub {
    /// harvest candidate claims from repo signals. emits candidates.json. NEVER writes claims.
    Suggest(SuggestArgs),
    /// ingest curated survivors.json. writes pending claims with archaeology_meta. requires debate.
    Commit(CommitArgs),
    /// remove archaeology claims by session (and optional agent). idempotent cleanup.
    Purge(PurgeArgs),
}

#[derive(Args)]
struct SuggestArgs {
    /// hard cap on candidates (default 10, ceiling 50)
    #[arg(long, default_value_t = 10)]
    max: usize,
    /// signal kinds to enable. v2.0 ships only `clms-claim-annotation`. (repeatable)
    #[arg(long)]
    source: Vec<String>,
    /// write candidates.json to this path (default: stdout)
    #[arg(long, short = 'o')]
    output: Option<std::path::PathBuf>,
}

#[derive(Args)]
struct CommitArgs {
    /// path to survivors.json from the debate phase
    #[arg(long = "from-plan")]
    from_plan: std::path::PathBuf,
    /// cap on committed claims (default 8)
    #[arg(long, default_value_t = 8)]
    keep: usize,
}

#[derive(Args)]
struct PurgeArgs {
    /// session stamp to match (e.g. backfill-2026-05-06T22:50:14Z)
    #[arg(long)]
    session: String,
    /// also restrict to this agent stamp (default: archaeology)
    #[arg(long)]
    agent: Option<String>,
}

const ARCHAEOLOGY_HELP: &str = r#"
EXAMPLES
  clms archaeology suggest                          # harvest candidates, print json to stdout
  clms archaeology suggest --max 5 -o cand.json     # write to file, cap at 5
  clms archaeology commit --from-plan survivors.json  # ingest curated survivors
  clms archaeology purge --session backfill-<ts>    # remove a backfill session

v2 CONTRACT
  archaeology is a CANDIDACY engine, not a verification engine.
  - phase 1 (suggest): byte-walks source for `// clms-claim:` and `# clms-claim:`
    annotations. bounded-N (default 10, ceiling 50). NEVER writes claims.
  - phase 2 (debate): orchestrator-agnostic. pi-subagents reference impl uses the
    `clms.judge` agent (.pi/agents/clms-judge.md). drop is structural default.
  - phase 3 (commit): writes `state: pending` claims with archaeology_meta.
    promotion to verified happens later via `clms verify`. always pending.

SEE ALSO
  docs/archaeology.md      full design + signal taxonomy + protocol contract
  clms verify              promote pending claims after archaeology commits them
  clms context --exclude-agent archaeology   filter backfill from live ctx
"#;

const SCHEMA_HELP: &str = r#"
EXAMPLES
  clms schema                          # human-readable schema overview
  clms --format ai schema              # full json schema for agent self-discovery

NOTES
  emits the requirement matrix per evidence method (mirrors what `verify`
  enforces at runtime), plus enum values for states, confidence tiers, edge
  types, and output formats. cache it once per agent session and you can
  build valid `verify` invocations without trial-and-error.
"#;

#[derive(Args)]
struct AddArgs {
    text: String,
    #[arg(long)]
    tag: Vec<String>,
    /// claims this one is built on. if any of these are refuted later, this one is auto-flagged suspect.
    #[arg(long = "depends-on")]
    depends_on: Vec<u64>,
    /// claim id this one is testing (neutral edge, outcome decides if it supports/refutes).
    #[arg(long)]
    tests: Vec<u64>,
    /// mark this claim explicitly as unverifiable (subjective, future-dependent, etc).
    #[arg(long)]
    unverifiable: bool,
    #[arg(long, env = "CLAIMS_AGENT")]
    agent: Option<String>,
    #[arg(long, env = "CLAIMS_SESSION")]
    session: Option<String>,
    /// override the auto-stamped git sha. for backfill / archaeology workflows.
    #[arg(long = "git-sha")]
    git_sha_override: Option<String>,
    /// override the auto-stamped created_at timestamp (RFC3339). for backfill / archaeology workflows.
    #[arg(long = "created-at")]
    created_at_override: Option<String>,
}

#[derive(Args)]
struct VerifyArgs {
    /// seq number or ulid
    id: String,
    #[arg(long)]
    method: String,
    #[arg(long = "ref")]
    r#ref: String,
    #[arg(long)]
    note: Option<String>,
    #[arg(long)]
    p_value: Option<f64>,
    #[arg(long)]
    sample_size: Option<u64>,
    #[arg(long)]
    test_type: Option<String>,
    #[arg(long)]
    exit_code: Option<i32>,
    #[arg(long)]
    quote: Option<String>,
    #[arg(long = "from")]
    from: Vec<u64>,
    /// shell command that produced this evidence. enables `claims rerun <id>` later.
    #[arg(long)]
    cmd: Option<String>,
    /// required when this --ref's content has changed since a prior evidence entry on the same claim.
    #[arg(long)]
    acknowledge_drift: bool,
}

#[derive(Args)]
struct RerunArgs {
    id: String,
    /// also acknowledge drift if the original test file's content changed.
    #[arg(long)]
    acknowledge_drift: bool,
}

#[derive(Args)]
struct RefuteArgs {
    id: String,
    #[arg(long)]
    by: u64,
    #[arg(long)]
    reason: String,
    /// also flag every transitive dependent as suspect.
    #[arg(long)]
    cascade: bool,
}

fn detect_early_format() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--format" {
            if let Some(next) = args.get(i + 1) {
                return Some(next.clone());
            }
        }
        if let Some(rest) = a.strip_prefix("--format=") {
            return Some(rest.to_string());
        }
        i += 1;
    }
    std::env::var("CLAIMS_FORMAT").ok().filter(|s| !s.is_empty())
}

fn is_ai_format(s: &Option<String>) -> bool {
    s.as_deref()
        .map(|x| x.eq_ignore_ascii_case("ai"))
        .unwrap_or(false)
}

fn emit_json_error(err: &str, kind: &str, code: i32, extras: serde_json::Value) {
    let mut env = serde_json::json!({
        "error": err,
        "kind": kind,
        "code": code,
    });
    if let serde_json::Value::Object(map) = extras {
        if let serde_json::Value::Object(out) = &mut env {
            for (k, v) in map {
                out.insert(k, v);
            }
        }
    }
    eprintln!("{}", env);
}

fn clap_err_extras(e: &clap::Error) -> (String, serde_json::Value) {
    use clap::error::ContextKind;
    let mut field: Option<String> = None;
    for (ck, cv) in e.context() {
        if matches!(ck, ContextKind::InvalidArg) {
            field = Some(cv.to_string());
            break;
        }
    }
    let kind_dbg = format!("{:?}", e.kind());
    let msg = e
        .to_string()
        .lines()
        .next()
        .unwrap_or("clap parse error")
        .trim_start_matches("error: ")
        .to_string();
    let mut extras = serde_json::Map::new();
    extras.insert("clap_kind".into(), serde_json::Value::String(kind_dbg));
    if let Some(f) = field {
        extras.insert("field".into(), serde_json::Value::String(f));
    }
    (msg, serde_json::Value::Object(extras))
}

fn main() {
    let early_fmt = detect_early_format();
    let ai = is_ai_format(&early_fmt);

    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => e.exit(),
                _ => {
                    if ai {
                        let (msg, extras) = clap_err_extras(&e);
                        emit_json_error(&msg, "clap", 2, extras);
                        std::process::exit(2);
                    } else {
                        e.exit();
                    }
                }
            }
        }
    };

    if let Err(e) = run(cli) {
        let msg = format!("{:#}", e);
        if ai {
            emit_json_error(&msg, "runtime", 1, serde_json::json!({}));
        } else {
            eprintln!("Error: {}", msg);
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let fmt = OutputFormat::parse(&cli.format)?;
    let cwd = cli.dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap());
    let mut store = Store::open_or_init(&cwd)?;

    match cli.cmd {
        Cmd::Add(args) => cmd_add(&mut store, args, fmt),
        Cmd::Verify(args) => cmd_verify(&mut store, args, fmt),
        Cmd::Refute(args) => cmd_refute(&mut store, args, fmt),
        Cmd::Show { id } => cmd_show(&store, id, fmt),
        Cmd::Timeline { tag, exclude_agent } => {
            print!(
                "{}",
                output::render_timeline(&store, fmt, tag.as_deref(), &exclude_agent)?
            );
            Ok(())
        }
        Cmd::Context { tag, exclude_agent } => {
            print!(
                "{}",
                output::render_context(&store, tag.as_deref(), fmt, &exclude_agent)?
            );
            Ok(())
        }
        Cmd::Suspect { exclude_agent } => cmd_suspect(&store, fmt, &exclude_agent),
        Cmd::Reindex => {
            let n = store.reindex_all()?;
            println!("reindexed {} claims", n);
            Ok(())
        }
        Cmd::Rerun(args) => cmd_rerun(&mut store, args, fmt),
        Cmd::DiffEvidence { id } => cmd_diff_evidence(&store, id, fmt),
        Cmd::HelpAll => cmd_help_all(),
        Cmd::Schema => cmd_schema(fmt),
        Cmd::InstallAgents { force, dry_run } => cmd_install_agents(fmt, force, dry_run),
        Cmd::Archaeology { sub } => {
            let s = match sub {
                ArchaeologySub::Suggest(a) => archaeology::Sub::Suggest(archaeology::SuggestArgs {
                    max: a.max,
                    source: a.source,
                    output: a.output,
                }),
                ArchaeologySub::Commit(a) => archaeology::Sub::Commit(archaeology::CommitArgs {
                    from_plan: a.from_plan,
                    keep: a.keep,
                }),
                ArchaeologySub::Purge(a) => archaeology::Sub::Purge(archaeology::PurgeArgs {
                    session: a.session,
                    agent: a.agent,
                }),
            };
            archaeology::dispatch(s, &mut store, fmt)
        }
    }
}

fn schema_value() -> serde_json::Value {
    serde_json::json!({
        "version": "2.0",
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
        "evidence_methods": {
            "stat-test": {
                "required": ["ref", "p_value", "sample_size", "test_type"],
                "recommended": ["cmd"],
                "confidence": "empirical",
                "notes": "file at --ref is content-hashed for drift detection"
            },
            "code-test": {
                "required": ["ref", "exit_code"],
                "recommended": ["cmd"],
                "confidence": "empirical",
                "notes": "file at --ref is content-hashed for drift detection"
            },
            "observed": {
                "required": ["ref"],
                "recommended": [],
                "confidence": "observed",
                "notes": "runtime data; --ref is path/url/hash"
            },
            "documented": {
                "required": ["ref", "quote"],
                "recommended": [],
                "confidence": "documented",
                "notes": "--ref is the doc url; --quote is exact text"
            },
            "derived": {
                "required": ["from (>=2)"],
                "recommended": [],
                "confidence": "derived",
                "notes": "inference from at least 2 other claim ids"
            }
        },
        "env_vars": {
            "CLAIMS_FORMAT":  "default output format (default|human|ai)",
            "CLAIMS_DIR":     "working directory containing .claims/",
            "CLAIMS_AGENT":   "auto-stamp every write with this agent name",
            "CLAIMS_SESSION": "auto-stamp every write with this session id"
        }
    })
}

fn cmd_schema(fmt: OutputFormat) -> Result<()> {
    if matches!(fmt, OutputFormat::Ai) {
        println!("{}", serde_json::to_string(&schema_value())?);
        return Ok(());
    }
    let s = schema_value();
    println!("clms schema v{}", s["version"].as_str().unwrap_or("?"));
    println!();
    println!("states:           {}", s["states"]);
    println!("confidence tiers: derived < documented < observed < empirical");
    println!("edge types:       {}", s["edge_types"]);
    println!("output formats:   {}", s["output_formats"]);
    println!();
    println!("evidence method requirements:");
    if let Some(map) = s["evidence_methods"].as_object() {
        for (name, spec) in map {
            let req = spec["required"].to_string();
            let conf = spec["confidence"].as_str().unwrap_or("?");
            println!("  {:<11} required={} → {}", name, req, conf);
        }
    }
    println!();
    println!("env vars: CLAIMS_FORMAT, CLAIMS_DIR, CLAIMS_AGENT, CLAIMS_SESSION");
    println!("under --format ai: errors emit json envelope on stderr (run `clms --format ai schema` for full spec)");
    Ok(())
}

// embedded at compile time so `cargo install` / homebrew binaries ship the
// agents without needing a source tree. updates require a rebuild.
const JUDGE_AGENT_MD: &str = include_str!("../.pi/agents/clms-judge.md");
const PROPOSER_AGENT_MD: &str = include_str!("../.pi/agents/clms-proposer.md");

fn cmd_install_agents(fmt: OutputFormat, force: bool, dry_run: bool) -> Result<()> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    let agents_dir = std::path::PathBuf::from(&home).join(".pi/agent/agents/clms");
    let judge_path = agents_dir.join("judge.md");
    let proposer_path = agents_dir.join("proposer.md");
    let mut written = Vec::new();
    let mut skipped = Vec::new();

    if !dry_run {
        std::fs::create_dir_all(&agents_dir).map_err(|e| anyhow!("mkdir {}: {}", agents_dir.display(), e))?;
    }
    for (path, body, label) in [
        (&judge_path, JUDGE_AGENT_MD, "clms.judge"),
        (&proposer_path, PROPOSER_AGENT_MD, "clms.proposer"),
    ] {
        let exists = path.exists();
        if exists && !force {
            skipped.push((label.to_string(), path.display().to_string()));
            continue;
        }
        if !dry_run {
            std::fs::write(path, body).map_err(|e| anyhow!("write {}: {}", path.display(), e))?;
        }
        written.push((label.to_string(), path.display().to_string()));
    }

    match fmt {
        OutputFormat::Ai => {
            let v = serde_json::json!({
                "action": "install-agents",
                "dry_run": dry_run,
                "force": force,
                "agents_dir": agents_dir.display().to_string(),
                "written": written.iter().map(|(a, p)| serde_json::json!({"agent": a, "path": p})).collect::<Vec<_>>(),
                "skipped": skipped.iter().map(|(a, p)| serde_json::json!({"agent": a, "path": p, "reason": "exists; pass --force to overwrite"})).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        _ => {
            if dry_run {
                println!("dry-run: would install to {}", agents_dir.display());
            } else {
                println!("installed to {}", agents_dir.display());
            }
            for (a, p) in &written {
                println!("  written  {} -> {}", a, p);
            }
            for (a, p) in &skipped {
                println!("  skipped  {} -> {} (exists; pass --force to overwrite)", a, p);
            }
            if !dry_run && !written.is_empty() {
                println!();
                println!("verify with: subagent({{ action: \"list\" }})  // expect clms.judge + clms.proposer");
            }
        }
    }
    Ok(())
}

fn cmd_help_all() -> Result<()> {
    use clap::CommandFactory;
    let mut top = Cli::command();
    println!("# top-level\n");
    top.print_long_help().ok();
    println!("\n");
    let names: Vec<String> = top
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .filter(|n| n != "help-all" && n != "help")
        .collect();
    for name in names {
        if let Some(sub) = top.find_subcommand_mut(&name) {
            println!("\n# {}\n", name);
            sub.print_long_help().ok();
            println!("\n");
        }
    }
    Ok(())
}

fn build_add_edges(depends_on: &[u64], tests: &[u64]) -> Vec<Edge> {
    let mut edges = Vec::with_capacity(depends_on.len() + tests.len());
    for d in depends_on {
        edges.push(Edge {
            r#type: EdgeType::DependsOn,
            to_seq: *d,
        });
    }
    for t in tests {
        edges.push(Edge {
            r#type: EdgeType::Tests,
            to_seq: *t,
        });
    }
    edges
}

fn parse_created_at(s: &str) -> Result<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow!("--created-at must be RFC3339 (e.g. 2024-01-15T12:34:56Z): {}", e))
}

fn cmd_add(store: &mut Store, a: AddArgs, fmt: OutputFormat) -> Result<()> {
    if a.text.trim().is_empty() {
        return Err(anyhow!("claim text cannot be empty"));
    }
    for d in a.depends_on.iter().chain(a.tests.iter()) {
        if store.read_claim(*d).is_err() {
            return Err(anyhow!("referenced claim #{} does not exist", d));
        }
    }
    let stamp_at = match a.created_at_override.as_deref() {
        Some(s) => parse_created_at(s)?,
        None => Utc::now(),
    };
    let stamp_sha = a.git_sha_override.clone().or_else(git::current_sha);
    let mut claim = Claim {
        schema_version: SCHEMA_VERSION.into(),
        id: Ulid::new(),
        seq: store.next_seq()?,
        text: a.text,
        state: if a.unverifiable {
            State::Unverifiable
        } else {
            State::Pending
        },
        tags: a.tag,
        evidence: vec![],
        edges: build_add_edges(&a.depends_on, &a.tests),
        agent: a.agent,
        session: a.session,
        git_sha: stamp_sha,
        created_at: stamp_at,
        updated_at: stamp_at,
        content_hash: None,
        archaeology_meta: None,
    };
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}

fn build_evidence(a: &VerifyArgs, m: EvidenceMethod) -> Evidence {
    Evidence {
        method: m,
        r#ref: a.r#ref.clone(),
        note: a.note.clone(),
        p_value: a.p_value,
        sample_size: a.sample_size,
        test_type: a.test_type.clone(),
        exit_code: a.exit_code,
        quote: a.quote.clone(),
        from_claims: a.from.clone(),
        ref_hash: hash_ref_if_local(&a.r#ref),
        cmd: a.cmd.clone(),
        stdout_hash: None,
        recorded_at: Utc::now(),
    }
}

fn cmd_verify(store: &mut Store, a: VerifyArgs, fmt: OutputFormat) -> Result<()> {
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    if claim.state == State::Refuted {
        return Err(anyhow!(
            "claim #{} is refuted; write a new claim instead of re-verifying",
            seq
        ));
    }
    let m = EvidenceMethod::parse(&a.method)?;
    let ev = build_evidence(&a, m);
    validate_evidence(&ev)?;
    if let Some((prior_hash, prior_at)) = detect_drift(&claim, &ev.r#ref, &ev.ref_hash) {
        if !a.acknowledge_drift {
            return Err(anyhow!(
                "evidence drift detected on '{}':\n  prior hash: {} (recorded {})\n  new hash:   {}\n\n\
the file behind this --ref has changed since prior evidence on this claim. \
if this iteration is intentional, re-run with --acknowledge-drift. if the \
result contradicts the prior conclusion, you should write a new claim and \
refute this one instead.",
                ev.r#ref,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    claim.evidence.push(ev);
    claim.state = State::Verified;
    claim.updated_at = Utc::now();
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}

fn cmd_rerun(store: &mut Store, a: RerunArgs, fmt: OutputFormat) -> Result<()> {
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    if claim.state == State::Refuted {
        return Err(anyhow!(
            "claim #{} is refuted; rerun is not meaningful. write a new claim.",
            seq
        ));
    }
    let prior = latest_runnable_evidence(&claim).ok_or_else(|| {
        anyhow!(
            "claim #{} has no runnable evidence (need code-test or stat-test with stored --cmd)",
            seq
        )
    })?;
    let cmd = prior.cmd.clone().unwrap();
    let r#ref = prior.r#ref.clone();
    let method = prior.method;
    println!("rerunning: {}", cmd);
    let (exit_code, stdout) = run_cmd(&cmd)?;
    let new_ref_hash = hash_ref_if_local(&r#ref);
    let stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());

    if !a.acknowledge_drift {
        if let Some((prior_hash, prior_at)) = detect_drift(&claim, &r#ref, &new_ref_hash) {
            return Err(anyhow!(
                "test file '{}' has changed since prior evidence:\n  prior hash: {} (recorded {})\n  new hash:   {}\n\nrun with --acknowledge-drift to record this rerun anyway.",
                r#ref,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                new_ref_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }

    let ev = Evidence {
        method,
        r#ref,
        note: Some(format!("rerun via `clms rerun {}`", seq)),
        p_value: None,
        sample_size: None,
        test_type: None,
        exit_code: Some(exit_code),
        quote: None,
        from_claims: vec![],
        ref_hash: new_ref_hash,
        cmd: Some(cmd),
        stdout_hash,
        recorded_at: Utc::now(),
    };
    claim.evidence.push(ev);
    claim.updated_at = Utc::now();
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}

fn evidence_extras_diff(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " p={} n={}",
            e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into())
        ),
        EvidenceMethod::CodeTest => format!(" exit={}", e.exit_code.unwrap_or(-1)),
        _ => String::new(),
    }
}

fn diff_evidence_ai(claim: &Claim) -> Result<String> {
    let arr: Vec<_> = claim
        .evidence
        .iter()
        .map(|e| {
            serde_json::json!({
                "recorded_at": e.recorded_at.to_rfc3339(),
                "method": e.method.as_str(),
                "ref": e.r#ref,
                "ref_hash": e.ref_hash,
                "stdout_hash": e.stdout_hash,
                "p_value": e.p_value,
                "sample_size": e.sample_size,
                "exit_code": e.exit_code,
                "note": e.note,
            })
        })
        .collect();
    Ok(serde_json::to_string(&arr)?)
}

fn print_diff_row(i: usize, e: &Evidence) {
    let h = e.ref_hash.as_deref().map(|s| &s[..12.min(s.len())]).unwrap_or("-");
    println!(
        "  [{}] {}  [{}] ref={} hash={}{}",
        i + 1,
        e.recorded_at.format("%Y-%m-%d %H:%M:%S"),
        e.method.as_str(),
        e.r#ref,
        h,
        evidence_extras_diff(e),
    );
    if let Some(n) = &e.note {
        println!("      note: {}", n);
    }
}

fn cmd_diff_evidence(store: &Store, id: String, fmt: OutputFormat) -> Result<()> {
    let seq = store.resolve(&id)?;
    let claim = store.read_claim(seq)?;
    if matches!(fmt, OutputFormat::Ai) {
        println!("{}", diff_evidence_ai(&claim)?);
        return Ok(());
    }
    println!("#{} evidence evolution ({} entries)", seq, claim.evidence.len());
    for (i, e) in claim.evidence.iter().enumerate() {
        print_diff_row(i, e);
    }
    Ok(())
}

fn refute_evidence(by: u64, reason: String) -> Evidence {
    Evidence {
        method: EvidenceMethod::Derived,
        r#ref: format!("claim:#{}", by),
        note: Some(reason),
        p_value: None,
        sample_size: None,
        test_type: None,
        exit_code: None,
        quote: None,
        from_claims: vec![by],
        ref_hash: None,
        cmd: None,
        stdout_hash: None,
        recorded_at: Utc::now(),
    }
}

fn render_cascade_note(store: &mut Store, seq: u64, cascade: bool) -> Result<String> {
    if cascade {
        let affected = cascade_suspect(store, seq)?;
        if affected.is_empty() {
            return Ok(String::new());
        }
        return Ok(format!(
            "\n{} {} dependent claim(s) auto-flagged suspect: {:?}\n",
            "cascade:".yellow().bold(),
            affected.len(),
            affected
        ));
    }
    let dependents = store.transitive_dependents(seq)?;
    if dependents.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "\n{} {} dependent claim(s) NOT flagged. re-run with --cascade to auto-mark suspect: {:?}\n",
        "warning:".yellow(),
        dependents.len(),
        dependents
    ))
}

fn cmd_refute(store: &mut Store, a: RefuteArgs, fmt: OutputFormat) -> Result<()> {
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    let _replacement = store.read_claim(a.by)?;
    claim.state = State::Refuted;
    claim.edges.push(Edge {
        r#type: EdgeType::Refutes,
        to_seq: a.by,
    });
    claim.evidence.push(refute_evidence(a.by, a.reason));
    claim.updated_at = Utc::now();
    store.write_claim(&mut claim)?;

    let mut out = output::render_claim(&claim, store, fmt)?;
    out.push_str(&render_cascade_note(store, seq, a.cascade)?);
    print!("{}", out);
    Ok(())
}

fn cmd_show(store: &Store, id: String, fmt: OutputFormat) -> Result<()> {
    let seq = store.resolve(&id)?;
    let c = store.read_claim(seq)?;
    print!("{}", output::render_claim(&c, store, fmt)?);
    Ok(())
}

fn agent_excluded(c: &Claim, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    match &c.agent {
        Some(a) => exclude.iter().any(|e| e == a),
        None => false,
    }
}

fn cmd_suspect(store: &Store, fmt: OutputFormat, exclude_agent: &[String]) -> Result<()> {
    let seqs = store.all_seqs()?;
    let claims: Vec<Claim> = seqs
        .iter()
        .filter_map(|s| store.read_claim(*s).ok())
        .filter(|c| c.state == State::Suspect)
        .filter(|c| !agent_excluded(c, exclude_agent))
        .collect();
    if matches!(fmt, OutputFormat::Ai) {
        let arr: Vec<_> = claims
            .iter()
            .map(|c| {
                serde_json::json!({
                    "seq": c.seq,
                    "text": c.text,
                    "tags": c.tags,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
        return Ok(());
    }
    if claims.is_empty() {
        println!("no suspect claims.");
        return Ok(());
    }
    println!("suspect claims ({}):", claims.len());
    for c in claims {
        println!("  #{} {}", c.seq, c.text);
    }
    Ok(())
}
