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
  every existing claim that must hold for yours via --depends-on. for full
  command reference in one shot: `clms help-all`.
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
    },
    /// compact verified-only digest for stuffing into agent context.
    Context {
        #[arg(long)]
        tag: Option<String>,
    },
    /// list all currently-suspect claims (auto-flagged by cascading refutes).
    Suspect,
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
}

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let fmt = OutputFormat::parse(&cli.format)?;
    let cwd = cli.dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap());
    let mut store = Store::open_or_init(&cwd)?;

    match cli.cmd {
        Cmd::Add(args) => cmd_add(&mut store, args, fmt),
        Cmd::Verify(args) => cmd_verify(&mut store, args, fmt),
        Cmd::Refute(args) => cmd_refute(&mut store, args, fmt),
        Cmd::Show { id } => cmd_show(&store, id, fmt),
        Cmd::Timeline { tag } => {
            print!("{}", output::render_timeline(&store, fmt, tag.as_deref())?);
            Ok(())
        }
        Cmd::Context { tag } => {
            print!("{}", output::render_context(&store, tag.as_deref(), fmt)?);
            Ok(())
        }
        Cmd::Suspect => cmd_suspect(&store, fmt),
        Cmd::Reindex => {
            let n = store.reindex_all()?;
            println!("reindexed {} claims", n);
            Ok(())
        }
        Cmd::Rerun(args) => cmd_rerun(&mut store, args, fmt),
        Cmd::DiffEvidence { id } => cmd_diff_evidence(&store, id, fmt),
        Cmd::HelpAll => cmd_help_all(),
    }
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

fn cmd_add(store: &mut Store, a: AddArgs, fmt: OutputFormat) -> Result<()> {
    if a.text.trim().is_empty() {
        return Err(anyhow!("claim text cannot be empty"));
    }
    for d in a.depends_on.iter().chain(a.tests.iter()) {
        if store.read_claim(*d).is_err() {
            return Err(anyhow!("referenced claim #{} does not exist", d));
        }
    }
    let now = Utc::now();
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
        git_sha: git::current_sha(),
        created_at: now,
        updated_at: now,
        content_hash: None,
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

fn cmd_suspect(store: &Store, fmt: OutputFormat) -> Result<()> {
    let seqs = store.all_seqs()?;
    let claims: Vec<Claim> = seqs
        .iter()
        .filter_map(|s| store.read_claim(*s).ok())
        .filter(|c| c.state == State::Suspect)
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
