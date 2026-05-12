//! artifact-driven empirical evidence: command preflight/exit gates plus
//! population of method-specific fields from local json artifacts.

mod populate;

pub(crate) use populate::populate_empirical_artifact_fields;

use anyhow::{anyhow, Result};

use crate::models::{Evidence, EvidenceMethod};

/// shared exit-code gate for artifact-driven methods. both cmd_verify and
/// cmd_rerun must refuse non-zero exits because the json artifact at --ref
/// is only trustworthy when --cmd succeeded.
pub(crate) fn assert_artifact_cmd_zero(
    method: EvidenceMethod,
    cmd: &str,
    exit: i32,
    stdout: &[u8],
    ctx: &str,
) -> Result<()> {
    if exit == 0 {
        return Ok(());
    }
    let preview = String::from_utf8_lossy(&stdout[..stdout.len().min(200)]);
    let ellipsis = if stdout.len() > 200 { " ..." } else { "" };
    Err(anyhow!(
        "{}: {} --cmd exited {} (expected 0 so clms can parse the local artifact at --ref).\n  cmd:    {}\n  stdout: {:?}{}",
        ctx,
        method.as_str(),
        exit,
        cmd,
        preview,
        ellipsis,
    ))
}

pub(crate) fn preflight_artifact_driven_inputs(ev: &Evidence) -> Result<()> {
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
