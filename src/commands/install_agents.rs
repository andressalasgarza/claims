use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::output::OutputFormat;

// embedded at compile time so `cargo install` / homebrew binaries ship the
// agents without needing a source tree. updates require a rebuild. paths
// are relative to this file (src/commands/install_agents.rs).
const JUDGE_AGENT_MD: &str = include_str!("../../.pi/agents/clms-judge.md");
const PROPOSER_AGENT_MD: &str = include_str!("../../.pi/agents/clms-proposer.md");

struct AgentInstallSpec {
    agents_dir: PathBuf,
    targets: Vec<(PathBuf, &'static str, &'static str)>,
}

struct AgentInstallReport {
    written: Vec<(String, String)>,
    skipped: Vec<(String, String)>,
}

pub(crate) fn cmd_install_agents(fmt: OutputFormat, force: bool, dry_run: bool) -> Result<()> {
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
