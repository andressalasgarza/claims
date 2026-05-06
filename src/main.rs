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
use store::{cascade_suspect, hash_ref_if_local, validate_evidence, Store};
use ulid::Ulid;

#[derive(Parser)]
#[command(name = "claims", version, about = "append-only ledger of falsifiable claims, agent-optimized")]
struct Cli {
    #[arg(long, default_value = "default", env = "CLAIMS_FORMAT")]
    format: String,

    #[arg(long, env = "CLAIMS_DIR")]
    dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// log a new claim. exits non-zero if you forgot anything required.
    Add(AddArgs),
    /// attach evidence to a claim and mark it verified. method-specific fields are required, no soft warnings.
    Verify(VerifyArgs),
    /// mark a claim refuted. requires the replacing claim id.
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
    }
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
    claim.evidence.push(ev);
    claim.state = State::Verified;
    claim.updated_at = Utc::now();
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
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
