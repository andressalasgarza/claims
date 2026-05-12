mod archaeology;
mod cli;
mod git;
mod models;
mod output;
mod schema;
mod store;

use crate::cli::{AddArgs, ArchaeologySub, Cli, Cmd, RefuteArgs, RerunArgs, VerifyArgs};
use crate::schema::schema_value;

use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::Parser;
use colored::*;
use models::{
    BenchmarkMetric, Claim, ConfidenceTier, DataSource, Edge, EdgeType, Estimator, Evidence,
    EvidenceMethod, HypothesisTest, State, SCHEMA_VERSION,
};
use output::OutputFormat;
use std::path::PathBuf;
use store::{
    cascade_suspect, detect_dataset_drift, detect_drift, detect_rename, evidence_user_hash,
    hash_ref_if_local, latest_runnable_evidence, run_cmd, target_is_local, validate_evidence, Store,
};
use ulid::Ulid;


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
            refuse_in_repair_mode("reindex")?;
            let n = store.reindex_all()?;
            println!("reindexed {} claims", n);
            Ok(())
        }
        Cmd::Rerun(args) => cmd_rerun(&mut store, args, fmt),
        Cmd::DiffEvidence { id } => cmd_diff_evidence(&store, id, fmt),
        Cmd::Stats {
            tag,
            exclude_agent,
        } => {
            print!(
                "{}",
                output::render_stats(&store, fmt, tag.as_deref(), &exclude_agent)?
            );
            Ok(())
        }
        Cmd::HelpAll => cmd_help_all(),
        Cmd::Schema { target } => cmd_schema(target, fmt),
        Cmd::InstallAgents { force, dry_run } => cmd_install_agents(fmt, force, dry_run),
        Cmd::Archaeology { sub } => {
            match &sub {
                ArchaeologySub::Commit(_) => refuse_in_repair_mode("archaeology commit")?,
                ArchaeologySub::Purge(_) => refuse_in_repair_mode("archaeology purge")?,
                ArchaeologySub::Suggest(_) => {}
            }
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
    if let Some(map) = s["env_vars"].as_object() {
        let names: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
        println!("env vars: {}", names.join(", "));
    }
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
    refuse_in_repair_mode("add")?;
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
    // stamp backfilled=true whenever provenance overrides were actually used
    // (presence-checked above; CLAIMS_BACKFILL only authorizes them, doesn't
    // mark the record). future-me reading this claim can then see "this
    // git_sha and created_at were retroactively asserted, not auto-captured."
    let backfilled = a.created_at_override.is_some() || a.git_sha_override.is_some();
    let min_tier = a.min_tier.as_deref().map(ConfidenceTier::parse).transpose()?;
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
        integrity_mac: None,
        archaeology_meta: None,
        backfilled,
        min_tier,
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
    // single-owner flags: exactly one method owns each. driven by
    // MethodSpec::exclusive_flag so adding a new flag-bearing method needs
    // no changes here.
    let single_owner_flags: [(&str, bool); 4] = [
        ("target", a.target.is_some()),
        ("dataset", a.dataset.is_some()),
        ("metric", a.metric.is_some()),
        ("estimator", a.estimator.is_some()),
    ];
    for (flag_name, is_present) in single_owner_flags {
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
    // data-source is shared by stat-test (real|live samples) and benchmark
    // (eval set provenance). refuse it on every other method. cannot use
    // the single-owner mechanism above since two methods legitimately
    // accept it.
    if a.data_source.is_some()
        && !matches!(
            method,
            EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate
        )
    {
        return Err(anyhow!(
            "--data-source is only valid with --method stat-test, --method benchmark, or --method estimate (got --method {})",
            method.as_str()
        ));
    }
    Ok(())
}

fn build_evidence(a: &VerifyArgs, m: EvidenceMethod) -> Result<Evidence> {
    check_exclusive_flags(m, a)?;
    // data-source is parsed for stat-test, benchmark, and estimate (the
    // three methods that legitimately accept it). check_exclusive_flags has
    // already refused it on every other method, so a non-None data-source
    // here is guaranteed to belong to one of those three.
    let data_source = match (&m, &a.data_source) {
        (
            EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate,
            Some(s),
        ) => Some(DataSource::parse(s)?),
        _ => None,
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
        test_type: a.test_type,
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
        metric: a.metric,
        metric_value: a.metric_value,
        threshold: a.threshold,
        estimator: a.estimator,
        point_value: a.point_value,
        ci_lower: a.ci_lower,
        ci_upper: a.ci_upper,
        confidence_level: a.confidence_level,
        recorded_at: Utc::now(),
    })
}

fn read_local_json_artifact(method: EvidenceMethod, r#ref: &str) -> Result<serde_json::Value> {
    let path = std::path::Path::new(r#ref);
    if !path.is_file() {
        return Err(anyhow!(
            "{} requires --ref to be a local file produced by --cmd. '{}' does not exist as a local file after execution.",
            method.as_str(),
            r#ref,
        ));
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("read {} artifact '{}': {}", method.as_str(), r#ref, e))?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        anyhow!(
            "{} --ref '{}' must contain valid JSON so clms can derive the measured values from the artifact. {}",
            method.as_str(),
            r#ref,
            e,
        )
    })?;
    if !value.is_object() {
        return Err(anyhow!(
            "{} --ref '{}' must contain a top-level JSON object.",
            method.as_str(),
            r#ref,
        ));
    }
    Ok(value)
}

fn artifact_req_str<'a>(value: &'a serde_json::Value, method: EvidenceMethod, r#ref: &str, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or_else(|| {
        anyhow!(
            "{} artifact '{}' is missing string field '{}'.",
            method.as_str(),
            r#ref,
            key,
        )
    })
}

fn artifact_req_f64(value: &serde_json::Value, method: EvidenceMethod, r#ref: &str, key: &str) -> Result<f64> {
    value[key].as_f64().ok_or_else(|| {
        anyhow!(
            "{} artifact '{}' is missing numeric field '{}'.",
            method.as_str(),
            r#ref,
            key,
        )
    })
}

fn artifact_req_u64(value: &serde_json::Value, method: EvidenceMethod, r#ref: &str, key: &str) -> Result<u64> {
    value[key].as_u64().ok_or_else(|| {
        anyhow!(
            "{} artifact '{}' is missing integer field '{}'.",
            method.as_str(),
            r#ref,
            key,
        )
    })
}

fn reject_artifact_mismatch(method: EvidenceMethod, r#ref: &str, flag: &str, cli: &str, artifact: &str) -> Result<()> {
    Err(anyhow!(
        "{} artifact '{}' disagrees with --{}: cli gave '{}', artifact says '{}'. clms now treats the artifact as the source of truth; fix the script or drop the stale flag.",
        method.as_str(),
        r#ref,
        flag,
        cli,
        artifact,
    ))
}

fn maybe_match_str(method: EvidenceMethod, r#ref: &str, flag: &str, cli: Option<&str>, artifact: &str) -> Result<()> {
    if let Some(cli) = cli {
        if cli != artifact {
            return reject_artifact_mismatch(method, r#ref, flag, cli, artifact);
        }
    }
    Ok(())
}

fn maybe_match_f64(method: EvidenceMethod, r#ref: &str, flag: &str, cli: Option<f64>, artifact: f64) -> Result<()> {
    if let Some(cli) = cli {
        if cli != artifact {
            return reject_artifact_mismatch(
                method,
                r#ref,
                flag,
                &cli.to_string(),
                &artifact.to_string(),
            );
        }
    }
    Ok(())
}

fn maybe_match_u64(method: EvidenceMethod, r#ref: &str, flag: &str, cli: Option<u64>, artifact: u64) -> Result<()> {
    if let Some(cli) = cli {
        if cli != artifact {
            return reject_artifact_mismatch(
                method,
                r#ref,
                flag,
                &cli.to_string(),
                &artifact.to_string(),
            );
        }
    }
    Ok(())
}

fn populate_stat_test_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let test_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "test_type")?;
    let parsed_test = HypothesisTest::parse(test_name)?;
    let p = artifact_req_f64(&doc, ev.method, &ev.r#ref, "p_value")?;
    let n = artifact_req_u64(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(src_name)?;

    maybe_match_str(ev.method, &ev.r#ref, "test-type", ev.test_type.map(|t| t.as_str()), parsed_test.as_str())?;
    maybe_match_f64(ev.method, &ev.r#ref, "p-value", ev.p_value, p)?;
    maybe_match_u64(ev.method, &ev.r#ref, "sample-size", ev.sample_size, n)?;
    maybe_match_str(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str()), parsed_src.as_str())?;

    ev.test_type = Some(parsed_test);
    ev.p_value = Some(p);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

fn populate_benchmark_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let metric_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "metric")?;
    let parsed_metric = BenchmarkMetric::parse(metric_name)?;
    let metric_value = artifact_req_f64(&doc, ev.method, &ev.r#ref, "metric_value")?;
    let n = artifact_req_u64(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(src_name)?;

    maybe_match_str(ev.method, &ev.r#ref, "metric", ev.metric.map(|m| m.as_str()), parsed_metric.as_str())?;
    maybe_match_f64(ev.method, &ev.r#ref, "metric-value", ev.metric_value, metric_value)?;
    maybe_match_u64(ev.method, &ev.r#ref, "sample-size", ev.sample_size, n)?;
    maybe_match_str(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str()), parsed_src.as_str())?;

    ev.metric = Some(parsed_metric);
    ev.metric_value = Some(metric_value);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

fn populate_estimate_from_artifact(ev: &mut Evidence) -> Result<()> {
    let doc = read_local_json_artifact(ev.method, &ev.r#ref)?;
    let estimator_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "estimator")?;
    let parsed_estimator = Estimator::parse(estimator_name)?;
    let point = artifact_req_f64(&doc, ev.method, &ev.r#ref, "point_value")?;
    let lo = artifact_req_f64(&doc, ev.method, &ev.r#ref, "ci_lower")?;
    let hi = artifact_req_f64(&doc, ev.method, &ev.r#ref, "ci_upper")?;
    let conf = artifact_req_f64(&doc, ev.method, &ev.r#ref, "confidence_level")?;
    let n = artifact_req_u64(&doc, ev.method, &ev.r#ref, "sample_size")?;
    let src_name = artifact_req_str(&doc, ev.method, &ev.r#ref, "data_source")?;
    let parsed_src = DataSource::parse(src_name)?;

    maybe_match_str(ev.method, &ev.r#ref, "estimator", ev.estimator.map(|m| m.as_str()), parsed_estimator.as_str())?;
    maybe_match_f64(ev.method, &ev.r#ref, "point-value", ev.point_value, point)?;
    maybe_match_f64(ev.method, &ev.r#ref, "ci-lower", ev.ci_lower, lo)?;
    maybe_match_f64(ev.method, &ev.r#ref, "ci-upper", ev.ci_upper, hi)?;
    maybe_match_f64(ev.method, &ev.r#ref, "confidence-level", ev.confidence_level, conf)?;
    maybe_match_u64(ev.method, &ev.r#ref, "sample-size", ev.sample_size, n)?;
    maybe_match_str(ev.method, &ev.r#ref, "data-source", ev.data_source.map(|d| d.as_str()), parsed_src.as_str())?;

    ev.estimator = Some(parsed_estimator);
    ev.point_value = Some(point);
    ev.ci_lower = Some(lo);
    ev.ci_upper = Some(hi);
    ev.confidence_level = Some(conf);
    ev.sample_size = Some(n);
    ev.data_source = Some(parsed_src);
    Ok(())
}

fn populate_empirical_artifact_fields(ev: &mut Evidence) -> Result<()> {
    match ev.method {
        EvidenceMethod::StatTest => populate_stat_test_from_artifact(ev),
        EvidenceMethod::Benchmark => populate_benchmark_from_artifact(ev),
        EvidenceMethod::Estimate => populate_estimate_from_artifact(ev),
        _ => Ok(()),
    }
}

fn preflight_artifact_driven_inputs(ev: &Evidence) -> Result<()> {
    let cmd = ev.cmd.as_deref().unwrap_or("").trim();
    if ev.r#ref.trim().is_empty() {
        return Err(anyhow!(
            "{} requires --ref <path-to-local-json-artifact>",
            ev.method.as_str()
        ));
    }
    if cmd.is_empty() {
        return Err(anyhow!(
            "{} requires --cmd \"<shell cmd that produces the local json artifact at --ref>\".",
            ev.method.as_str()
        ));
    }
    Ok(())
}

/// CLAIMS_REPAIR=1 disables claim-integrity verification in store::read_claim
/// so a human can recover from a corrupted .claims/ tree (forensic
/// forward-read). any *mutating* command run while repair is in effect would
/// carry the tampered state forward; in the worst case (rerun) it would
/// execute a tampered cmd field as shell. mutators bail at the top with this
/// helper.
fn refuse_in_repair_mode(op: &str) -> Result<()> {
    if std::env::var("CLAIMS_REPAIR").is_ok() {
        return Err(anyhow!(
            "{}: refusing under CLAIMS_REPAIR=1. repair mode is read-only — it skips claim-integrity verification for forensic recovery (show, diff-evidence, timeline, context). fix the corrupted records first, then unset CLAIMS_REPAIR before mutating state. without this guard, a tampered claim could be carried forward into add/verify/refute/rerun/reindex or archaeology mutation.", 
            op
        ));
    }
    Ok(())
}

fn cmd_verify(store: &mut Store, a: VerifyArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("verify")?;
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    if claim.state == State::Refuted {
        return Err(anyhow!(
            "claim #{} is refuted; write a new claim instead of re-verifying",
            seq
        ));
    }
    let m = EvidenceMethod::parse(&a.method)?;
    let mut ev = build_evidence(&a, m)?;
    if matches!(
        m,
        EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate
    ) {
        preflight_artifact_driven_inputs(&ev)?;
        let cmd = ev
            .cmd
            .as_deref()
            .expect("preflight guarantees --cmd presence for artifact-driven methods");
        eprintln!("executing: {}", cmd);
        let (actual_exit, stdout) = run_cmd(cmd)?;
        if actual_exit != 0 {
            let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
            return Err(anyhow!(
                "{} --cmd exited {} (expected 0). the script must succeed and produce the local artifact at --ref so clms can derive the measured values from that artifact.\n  cmd:    {}\n  stdout: {:?}{}",
                m.as_str(),
                actual_exit,
                cmd,
                preview,
                if stdout.len() > 200 { " ..." } else { "" },
            ));
        }
        ev.exit_code = Some(actual_exit);
        ev.stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
        ev.ref_hash = hash_ref_if_local(&ev.r#ref);
        populate_empirical_artifact_fields(&mut ev)?;
        validate_evidence(&ev, store, seq)?;
    } else {
        validate_evidence(&ev, store, seq)?;
        // loopback / private-network --target check for integration-test.
        // integration-test's whole point is hitting a real external system the
        // author does not control. localhost / 127.x / RFC1918 fail that. allow
        // the override (--allow-local) because dev/CI smoketests legitimately
        // probe local services, but make it explicit so reviewers can filter.
        if matches!(m, EvidenceMethod::IntegrationTest) {
            if let Some(t) = ev.target.as_deref() {
                if target_is_local(t) && !a.allow_local {
                    return Err(anyhow!(
                        "integration-test --target '{}' is a loopback / private-network address. integration-test must hit a real external system the author does not control; a service on your own machine cannot falsify that.\n\npass --allow-local if you intentionally want to record a local smoketest as integration evidence (lower-confidence by convention, but accepted), or use --method observed with a log artifact instead.",
                        t
                    ));
                }
            }
        }
        // execute --cmd at verify time for methods that take one. an agent can no
        // longer pass --exit-code 0 against a command that returns 1; clms runs
        // the command itself and stores the actual result. the user-supplied
        // --exit-code, if any, becomes a *predicted* value: a mismatch with the
        // actual value is a hard error before any state mutation. without this
        // pass, the falsifiability bar for prop/integration/replay was an honor
        // system since the contradiction only surfaced on `clms rerun` (which a
        // cheating agent never invokes).
        if matches!(
            m,
            EvidenceMethod::PropTest | EvidenceMethod::IntegrationTest | EvidenceMethod::ReplayTest
        ) {
            let cmd = ev
                .cmd
                .as_deref()
                .expect("validate_evidence guarantees --cmd presence for these methods");
            let claimed_exit = ev.exit_code;
            eprintln!("executing: {}", cmd);
            let (actual_exit, stdout) = run_cmd(cmd)?;
            if let Some(claimed) = claimed_exit {
                if claimed != actual_exit {
                    let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
                    return Err(anyhow!(
                        "--exit-code mismatch on claim #{}: you claimed {}, --cmd actually exited {}.\n  cmd:    {}\n  stdout: {:?}{}\n\nclms now executes --cmd at verify time and captures the actual exit_code. either fix --cmd, drop the (now-optional) --exit-code flag, or write a different claim if the contradiction is real.",
                        seq,
                        claimed,
                        actual_exit,
                        cmd,
                        preview,
                        if stdout.len() > 200 { " ..." } else { "" },
                    ));
                }
            }
            ev.exit_code = Some(actual_exit);
            ev.stdout_hash = Some(blake3::hash(&stdout).to_hex().to_string());
        }
    }
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
    if let Some((prior_ref, prior_at)) = detect_rename(&claim, &ev.r#ref, &ev.ref_hash) {
        if !a.acknowledge_drift {
            return Err(anyhow!(
                "ref rename detected: --ref '{}' has the same content_hash as prior evidence on claim #{} which used '{}' (recorded {}).\n\n\
the bytes are identical — this is a rename/copy/symlink of the same file under a new path, not new evidence. without this check the dedup (which keys on the ref *string*) would accept this as independent evidence and inflate the count for free. use the original ref, re-run with --acknowledge-drift if the renamed copy is intentional documentation, or write a new claim if it really refers to a distinct file.",
                ev.r#ref, seq, prior_ref, prior_at
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
    // min_tier gate: if the claim was stamped with a tier floor at write time
    // (orchestrator intent: "this is a science claim, demand empirical"),
    // refuse the verify if the method's tier is below the floor. uses the
    // METHODS table for tier lookup so it cannot drift from validation logic.
    if let Some(floor) = claim.min_tier {
        let ev_tier = ev.method.tier();
        if !ev_tier.is_at_least(floor) {
            use crate::models::METHODS;
            let allowed: Vec<&str> = METHODS
                .iter()
                .filter(|m| m.tier.is_at_least(floor))
                .map(|m| m.name)
                .collect();
            return Err(anyhow!(
                "claim #{} requires min_tier={} (got method '{}' at tier {}). allowed methods: {}.\n\nthis claim was stamped with a tier floor at creation time (likely by an orchestrator declaring it a science claim). either supply evidence at or above the floor, or write a different claim if the lower-tier evidence is the actual finding.",
                seq,
                floor.as_str(),
                m.as_str(),
                ev_tier.as_str(),
                allowed.join(" | "),
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
    refuse_in_repair_mode("rerun")?;
    let seq = store.resolve(&a.id)?;
    let mut claim = store.read_claim(seq)?;
    if claim.state == State::Refuted {
        return Err(anyhow!(
            "claim #{} is refuted; rerun is not meaningful. write a new claim.",
            seq
        ));
    }
    let prior = latest_runnable_evidence(&claim).ok_or_else(|| {
        use crate::models::METHODS;
        let runnable_names: Vec<&str> = METHODS
            .iter()
            .filter(|m| m.runnable)
            .map(|m| m.name)
            .collect();
        anyhow!(
            "claim #{} has no runnable evidence (need one of {} with stored --cmd)",
            seq,
            runnable_names.join(", ")
        )
    })?;
    let cmd = prior.cmd.clone().unwrap();
    let r#ref = prior.r#ref.clone();
    let method = prior.method;
    let prior_target = prior.target.clone();
    let prior_dataset = prior.dataset.clone();
    let prior_threshold = prior.threshold;
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
    eprintln!("rerunning: {}", cmd);
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
    let mut ev = Evidence {
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
        data_source: None,
        metric: None,
        metric_value: None,
        threshold: prior_threshold,
        estimator: None,
        point_value: None,
        ci_lower: None,
        ci_upper: None,
        confidence_level: None,
        recorded_at: Utc::now(),
    };
    if matches!(
        method,
        EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate
    ) {
        if exit_code != 0 {
            let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
            return Err(anyhow!(
                "rerun failed for {} on claim #{}: --cmd exited {} (expected 0 so clms can parse the local artifact at --ref).\n  cmd:    {}\n  stdout: {:?}{}",
                method.as_str(),
                seq,
                exit_code,
                ev.cmd.as_deref().unwrap_or(""),
                preview,
                if stdout.len() > 200 { " ..." } else { "" },
            ));
        }
        populate_empirical_artifact_fields(&mut ev)?;
        validate_evidence(&ev, store, seq)?;
    }
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
        metric: None,
        metric_value: None,
        threshold: None,
        estimator: None,
        point_value: None,
        ci_lower: None,
        ci_upper: None,
        confidence_level: None,
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
    refuse_in_repair_mode("refute")?;
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
