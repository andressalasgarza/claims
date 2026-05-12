#[macro_use]
mod macros;

mod archaeology;
mod cli;
mod commands;
mod error_handling;
mod git;
mod models;
mod output;
mod schema;
mod store;

use crate::cli::{ArchaeologySub, Cli, Cmd};
use crate::commands::add::cmd_add;
use crate::commands::diff::cmd_diff_evidence;
use crate::commands::help_all::cmd_help_all;
use crate::commands::install_agents::cmd_install_agents;
use crate::commands::refute::cmd_refute;
use crate::commands::rerun::cmd_rerun;
use crate::commands::schema_cli::cmd_schema;
use crate::commands::show::cmd_show;
use crate::commands::suspect::cmd_suspect;
use crate::commands::util::refuse_in_repair_mode;
use crate::commands::verify::cmd_verify;
use crate::error_handling::{clap_err_extras, detect_early_format, emit_json_error, is_ai_format};

use anyhow::Result;
use clap::Parser;
use output::OutputFormat;
use store::Store;


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

/// the reindex sub-command lives in main.rs (no commands/reindex.rs file)
/// because its body is a one-liner + the refuse-in-repair guard. extracted
/// here so `run` doesn't carry the inline `?` + println chain.
fn cmd_reindex(store: &mut Store) -> Result<()> {
    refuse_in_repair_mode("reindex")?;
    let n = store.reindex_all()?;
    println!("reindexed {} claims", n);
    Ok(())
}

/// render-only commands (timeline / context / stats) share the same shape:
/// call an output::render_* fn, print the string, return Ok. dispatched
/// here to keep `run`'s match flat.
fn cmd_render_pure(
    cmd: Cmd, store: &Store, fmt: OutputFormat,
) -> Result<()> {
    let out = match cmd {
        Cmd::Timeline { tag, exclude_agent } => {
            output::render_timeline(store, fmt, tag.as_deref(), &exclude_agent)?
        }
        Cmd::Context { tag, exclude_agent } => {
            output::render_context(store, tag.as_deref(), fmt, &exclude_agent)?
        }
        Cmd::Stats { tag, exclude_agent } => {
            output::render_stats(store, fmt, tag.as_deref(), &exclude_agent)?
        }
        _ => unreachable!("cmd_render_pure called with non-render command"),
    };
    print!("{}", out);
    Ok(())
}

/// archaeology dispatch: refuse mutators under CLAIMS_REPAIR, then forward
/// to the archaeology module after translating the cli sub-enum to the
/// module-internal one. extracted so `run` carries one arm for the whole
/// archaeology surface.
fn cmd_archaeology(
    sub: ArchaeologySub, store: &mut Store, fmt: OutputFormat,
) -> Result<()> {
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
    archaeology::dispatch(s, store, fmt)
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
        Cmd::Suspect { exclude_agent } => cmd_suspect(&store, fmt, &exclude_agent),
        Cmd::Reindex => cmd_reindex(&mut store),
        Cmd::Rerun(args) => cmd_rerun(&mut store, args, fmt),
        Cmd::DiffEvidence { id } => cmd_diff_evidence(&store, id, fmt),
        Cmd::HelpAll => cmd_help_all(),
        Cmd::Schema { target } => cmd_schema(target, fmt),
        Cmd::InstallAgents { force, dry_run } => cmd_install_agents(fmt, force, dry_run),
        Cmd::Archaeology { sub } => cmd_archaeology(sub, &mut store, fmt),
        c @ (Cmd::Timeline { .. } | Cmd::Context { .. } | Cmd::Stats { .. }) => {
            cmd_render_pure(c, &store, fmt)
        }
    }
}
