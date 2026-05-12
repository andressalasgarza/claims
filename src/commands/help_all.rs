use anyhow::Result;
use clap::CommandFactory;

use crate::cli::Cli;

/// dump top-level help + every subcommand's help in one go. used by
/// agents to context-stuff the entire CLI surface in a single shot.
pub(crate) fn cmd_help_all() -> Result<()> {
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
