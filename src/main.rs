mod archaeology;
mod cli;
mod commands;
mod error_handling;
mod git;
mod models;
mod output;
mod schema;
mod store;

use crate::cli::{ArchaeologySub, Cli, Cmd, RerunArgs, VerifyArgs};
use crate::commands::add::cmd_add;
use crate::commands::diff::cmd_diff_evidence;
use crate::commands::help_all::cmd_help_all;
use crate::commands::install_agents::cmd_install_agents;
use crate::commands::refute::cmd_refute;
use crate::commands::schema_cli::cmd_schema;
use crate::commands::show::cmd_show;
use crate::commands::suspect::cmd_suspect;
use crate::commands::util::{commit_evidence, refuse_in_repair_mode};
use crate::commands::verify::artifact::populate_empirical_artifact_fields;
use crate::commands::verify::cmd_verify;
use crate::error_handling::{clap_err_extras, detect_early_format, emit_json_error, is_ai_format};

use anyhow::{anyhow, Result};
use chrono::Utc;
use clap::Parser;
use models::{Evidence, EvidenceMethod, State};
use output::OutputFormat;
use store::{
    detect_drift, hash_ref_if_local, latest_runnable_evidence, run_cmd, validate_evidence, Store,
};


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


/// reject cli flags that are exclusive to a single method when used with the
/// wrong method. driven by `MethodSpec::exclusive_flag` so adding a new
/// flag-bearing method requires no changes here.
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
        commit_evidence(&mut claim, store)?;
        print!("{}", output::render_claim(&claim, store, fmt)?);
        return Err(anyhow!(
            "rerun contradicts prior evidence on claim #{}: stored exit_code={}, fresh exit_code={}.\nclaim flipped to suspect. write a new claim and refute this one if the contradiction is real, or investigate the rerun environment if it is spurious.",
            seq, prev, now,
        ));
    }
    commit_evidence(&mut claim, store)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}


