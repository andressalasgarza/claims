mod archaeology;
mod git;
mod models;
mod output;
mod schema;
mod store;

use crate::schema::schema_value;

use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use colored::*;
use models::{Claim, DataSource, Edge, EdgeType, Evidence, EvidenceMethod, State, SCHEMA_VERSION};
use output::OutputFormat;
use std::path::PathBuf;
use store::{
    cascade_suspect, detect_dataset_drift, detect_drift, evidence_user_hash,
    hash_ref_if_local, latest_runnable_evidence, run_cmd, validate_evidence, Store,
};
use ulid::Ulid;

const TOP_HELP: &str = include_str!("help_text/top.txt");

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

const ADD_HELP: &str = include_str!("help_text/add.txt");

const VERIFY_HELP: &str = include_str!("help_text/verify.txt");

const REFUTE_HELP: &str = include_str!("help_text/refute.txt");

const RERUN_HELP: &str = include_str!("help_text/rerun.txt");

const DIFF_HELP: &str = include_str!("help_text/diff.txt");

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
    output: Option<PathBuf>,
}

#[derive(Args)]
struct CommitArgs {
    /// path to survivors.json from the debate phase
    #[arg(long = "from-plan")]
    from_plan: PathBuf,
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

const ARCHAEOLOGY_HELP: &str = include_str!("help_text/archaeology.txt");

const SCHEMA_HELP: &str = include_str!("help_text/schema.txt");

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
    /// integration-test only: real external system probed (url/host/endpoint).
    /// the falsification surface must be auditable.
    #[arg(long)]
    target: Option<String>,
    /// replay-test only: path to real-world captured dataset (parquet/csv/jsonl).
    /// content-hashed at write time. synthetic data is not allowed.
    #[arg(long)]
    dataset: Option<String>,
    /// stat-test only: real | live. simulated/synthetic data is refused.
    #[arg(long)]
    data_source: Option<String>,
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
        Cmd::Schema { target } => cmd_schema(target, fmt),
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


fn cmd_schema(target: Option<String>, fmt: OutputFormat) -> Result<()> {
    if let Some(t) = target.as_deref() {
        match t {
            "methods" => {
                println!("{}", serde_json::to_string_pretty(&schema::methods_table())?);
                return Ok(());
            }
            other => {
                return Err(anyhow!(
                    "unknown schema target '{}'. valid: methods",
                    other
                ));
            }
        }
    }
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

struct AgentInstallSpec {
    agents_dir: PathBuf,
    targets: Vec<(PathBuf, &'static str, &'static str)>,
}

struct AgentInstallReport {
    written: Vec<(String, String)>,
    skipped: Vec<(String, String)>,
}

fn cmd_install_agents(fmt: OutputFormat, force: bool, dry_run: bool) -> Result<()> {
    let spec = resolve_agent_install_spec()?;
    if !dry_run {
        std::fs::create_dir_all(&spec.agents_dir)
            .map_err(|e| anyhow!("mkdir {}: {}", spec.agents_dir.display(), e))?;
    }
    let report = install_agent_files(&spec, dry_run, force)?;
    emit_install_report(fmt, &spec.agents_dir, dry_run, force, &report)
}

fn resolve_agent_install_spec() -> Result<AgentInstallSpec> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    let agents_dir = PathBuf::from(&home).join(".pi/agent/agents/clms");
    let judge_path = agents_dir.join("judge.md");
    let proposer_path = agents_dir.join("proposer.md");
    Ok(AgentInstallSpec {
        agents_dir,
        targets: vec![
            (judge_path, JUDGE_AGENT_MD, "clms.judge"),
            (proposer_path, PROPOSER_AGENT_MD, "clms.proposer"),
        ],
    })
}

fn install_agent_files(
    spec: &AgentInstallSpec, dry_run: bool, force: bool,
) -> Result<AgentInstallReport> {
    let mut report = AgentInstallReport { written: Vec::new(), skipped: Vec::new() };
    for (path, body, label) in &spec.targets {
        if path.exists() && !force {
            report.skipped.push((label.to_string(), path.display().to_string()));
            continue;
        }
        if !dry_run {
            std::fs::write(path, body)
                .map_err(|e| anyhow!("write {}: {}", path.display(), e))?;
        }
        report.written.push((label.to_string(), path.display().to_string()));
    }
    Ok(report)
}

fn emit_install_report(
    fmt: OutputFormat,
    agents_dir: &std::path::Path,
    dry_run: bool,
    force: bool,
    report: &AgentInstallReport,
) -> Result<()> {
    if matches!(fmt, OutputFormat::Ai) {
        let v = serde_json::json!({
            "action": "install-agents",
            "dry_run": dry_run,
            "force": force,
            "agents_dir": agents_dir.display().to_string(),
            "written": report.written.iter()
                .map(|(a, p)| serde_json::json!({"agent": a, "path": p}))
                .collect::<Vec<_>>(),
            "skipped": report.skipped.iter()
                .map(|(a, p)| serde_json::json!({
                    "agent": a, "path": p,
                    "reason": "exists; pass --force to overwrite"
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if dry_run {
        println!("dry-run: would install to {}", agents_dir.display());
    } else {
        println!("installed to {}", agents_dir.display());
    }
    for (a, p) in &report.written {
        println!("  written  {} -> {}", a, p);
    }
    for (a, p) in &report.skipped {
        println!("  skipped  {} -> {} (exists; pass --force to overwrite)", a, p);
    }
    if !dry_run && !report.written.is_empty() {
        println!();
        println!("verify with: subagent({{ action: \"list\" }})  // expect clms.judge + clms.proposer");
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
    // backfill gate: --created-at and --git-sha forge provenance metadata.
    // they are documented for archaeology workflows but were exposed on every
    // `clms add`, allowing any agent to stamp a claim with arbitrary git_sha
    // and created_at (e.g., year 9999, deadbeef..., a colleague's actual sha).
    // require CLAIMS_BACKFILL=1 to use them. archaeology constructs claims
    // directly via the Claim struct, not via cmd_add, so it is unaffected.
    let backfill_authorized = std::env::var("CLAIMS_BACKFILL").is_ok();
    if (a.created_at_override.is_some() || a.git_sha_override.is_some()) && !backfill_authorized {
        let mut flags = Vec::new();
        if a.created_at_override.is_some() {
            flags.push("--created-at");
        }
        if a.git_sha_override.is_some() {
            flags.push("--git-sha");
        }
        return Err(anyhow!(
            "refusing to backfill provenance: {} requires CLAIMS_BACKFILL=1.\n\nthese flags forge git_sha and created_at; they are intended for archaeology / historical reconstruction only. set CLAIMS_BACKFILL=1 in the environment to authorize, then re-run.",
            flags.join(" + "),
        ));
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

/// reject cli flags that are exclusive to a single method when used with the
/// wrong method. driven by `MethodSpec::exclusive_flag` so adding a new
/// flag-bearing method requires no changes here.
fn check_exclusive_flags(method: EvidenceMethod, a: &VerifyArgs) -> Result<()> {
    use crate::models::METHODS;
    let present_flags: [(&str, bool); 3] = [
        ("target", a.target.is_some()),
        ("dataset", a.dataset.is_some()),
        ("data-source", a.data_source.is_some()),
    ];
    for (flag_name, is_present) in present_flags {
        if !is_present {
            continue;
        }
        let owner = METHODS.iter().find(|m| m.exclusive_flag == Some(flag_name));
        match owner {
            Some(spec) if spec.variant == method => {} // ok: method owns this flag
            Some(spec) => {
                return Err(anyhow!(
                    "--{} is only valid with --method {} (got --method {})",
                    flag_name,
                    spec.name,
                    method.as_str()
                ));
            }
            None => {} // flag has no exclusivity constraint
        }
    }
    Ok(())
}

fn build_evidence(a: &VerifyArgs, m: EvidenceMethod) -> Result<Evidence> {
    check_exclusive_flags(m, a)?;
    let data_source = match (&m, &a.data_source) {
        (EvidenceMethod::StatTest, Some(s)) => Some(DataSource::parse(s)?),
        _ => None, // non-stat-test --data-source already rejected by check_exclusive_flags
    };
    let dataset_hash = a.dataset.as_deref().and_then(hash_ref_if_local);
    let cmd_hash = a
        .cmd
        .as_deref()
        .map(|c| blake3::hash(c.as_bytes()).to_hex().to_string());
    Ok(Evidence {
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
        cmd_hash,
        stdout_hash: None,
        target: a.target.clone(),
        dataset: a.dataset.clone(),
        dataset_hash,
        data_source,
        recorded_at: Utc::now(),
    })
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
    let ev = build_evidence(&a, m)?;
    validate_evidence(&ev, store, seq)?;
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
    if let Some((ds, prior_hash, prior_at)) =
        detect_dataset_drift(&claim, ev.dataset.as_deref(), &ev.dataset_hash)
    {
        if !a.acknowledge_drift {
            return Err(anyhow!(
                "dataset drift detected on '{}':\n  prior dataset_hash: {} (recorded {})\n  new dataset_hash:   {}\n\n\
the replay dataset bytes have changed since prior evidence on this claim, even though the dataset path is unchanged. \
an agent that swaps parquet/csv contents can defeat ref-string drift detection. if this iteration is intentional, \
re-run with --acknowledge-drift. if the new bytes contradict the prior conclusion, write a new claim and refute \
this one instead.",
                ds,
                &prior_hash[..16.min(prior_hash.len())],
                prior_at,
                ev.dataset_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("-"),
            ));
        }
    }
    // dedup: if the user-supplied evidence fields collide with an existing
    // entry, refuse. otherwise an agent can stack 5x identical `verify` calls
    // and inflate the apparent evidence count for free.
    let new_user_hash = evidence_user_hash(&ev);
    if claim
        .evidence
        .iter()
        .any(|prior| evidence_user_hash(prior) == new_user_hash)
    {
        return Err(anyhow!(
            "duplicate evidence: claim #{} already has an entry with the same method, ref, and parameters. add new information (a different ref, exit_code, target, dataset, etc.) or write a new claim.",
            seq
        ));
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
            "claim #{} has no runnable evidence (need prop-test, integration-test, replay-test, or stat-test with stored --cmd)",
            seq
        )
    })?;
    let cmd = prior.cmd.clone().unwrap();
    let r#ref = prior.r#ref.clone();
    let method = prior.method;
    let prior_target = prior.target.clone();
    let prior_dataset = prior.dataset.clone();
    let prior_data_source = prior.data_source;
    let prior_exit = prior.exit_code;
    // tamper-gate: re-hash the stored cmd and refuse to execute if the json was
    // edited since verify time. without this check, anyone who can write to a
    // .claims/*.json gets arbitrary code execution via `clms rerun`.
    let recomputed_cmd_hash = blake3::hash(cmd.as_bytes()).to_hex().to_string();
    match prior.cmd_hash.as_deref() {
        None => {
            return Err(anyhow!(
                "claim #{} has runnable evidence with no cmd_hash (pre-tamper-gate format). \
 re-verify with the current binary to record cmd_hash, then rerun.",
                seq
            ));
        }
        Some(prior_hash) if prior_hash != recomputed_cmd_hash => {
            return Err(anyhow!(
                "refusing to rerun: cmd has been tampered since verify time.\n  expected cmd_hash: {}\n  actual cmd_hash:   {}\n  cmd:               {:?}\n\nthe `cmd` field in the on-disk evidence record was edited after verify. \
rerun would execute attacker-controlled shell. write a new claim instead.",
                &prior_hash[..16.min(prior_hash.len())],
                &recomputed_cmd_hash[..16],
                cmd,
            ));
        }
        _ => {}
    }
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

    let new_dataset_hash = prior_dataset.as_deref().and_then(hash_ref_if_local);
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
        cmd_hash: Some(recomputed_cmd_hash),
        stdout_hash,
        target: prior_target,
        dataset: prior_dataset,
        dataset_hash: new_dataset_hash,
        data_source: prior_data_source,
        recorded_at: Utc::now(),
    };
    claim.evidence.push(ev);
    claim.updated_at = Utc::now();

    // contradiction-gate: if the rerun exit_code disagrees with what was
    // recorded at verify time, the claim's verified status is now suspect.
    // record the new evidence (audit trail), flip state, write, then exit 1.
    // without this, an agent can lie `--exit-code 0` on a script that exits
    // 1, and `rerun` faithfully captures exit=1 but the claim stays verified.
    let contradicted = match (prior_exit, exit_code) {
        (Some(prev), now) if prev != now => Some((prev, now)),
        _ => None,
    };
    if let Some((prev, now)) = contradicted {
        if matches!(claim.state, State::Verified | State::Pending) {
            claim.state = State::Suspect;
        }
        store.write_claim(&mut claim)?;
        print!("{}", output::render_claim(&claim, store, fmt)?);
        return Err(anyhow!(
            "rerun contradicts prior evidence on claim #{}: stored exit_code={}, fresh exit_code={}.\nclaim flipped to suspect. write a new claim and refute this one if the contradiction is real, or investigate the rerun environment if it is spurious.",
            seq, prev, now,
        ));
    }
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}

fn evidence_extras_diff(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " p={} n={} src={}",
            e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::PropTest => format!(" exit={}", e.exit_code.unwrap_or(-1)),
        EvidenceMethod::IntegrationTest => format!(
            " exit={} target={}",
            e.exit_code.unwrap_or(-1),
            e.target.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::ReplayTest => format!(
            " exit={} dataset={}",
            e.exit_code.unwrap_or(-1),
            e.dataset.as_deref().unwrap_or("?")
        ),
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
                "target": e.target,
                "dataset": e.dataset,
                "dataset_hash": e.dataset_hash,
                "data_source": e.data_source.map(|d| d.as_str()),
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
        cmd_hash: None,
        stdout_hash: None,
        target: None,
        dataset: None,
        dataset_hash: None,
        data_source: None,
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
    if a.by == seq {
        return Err(anyhow!(
            "refute: claim #{} cannot refute itself. self-refutation is incoherent — if the claim is wrong, write a different claim that contradicts it and refute via that.",
            seq
        ));
    }
    let mut claim = store.read_claim(seq)?;
    let replacement = store.read_claim(a.by)?;
    if !matches!(replacement.state, State::Verified) {
        return Err(anyhow!(
            "refute: --by #{} is {} (not verified). a refutation must replace a claim with a verified counter-claim. either verify #{} first or write a different replacement.",
            a.by,
            replacement.state.as_str(),
            a.by
        ));
    }
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

fn cmd_suspect(store: &Store, fmt: OutputFormat, exclude_agent: &[String]) -> Result<()> {
    let seqs = store.all_seqs()?;
    let claims: Vec<Claim> = seqs
        .iter()
        .map(|s| store.read_claim(*s))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|c| c.state == State::Suspect)
        .filter(|c| !output::agent_excluded(c, exclude_agent))
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
