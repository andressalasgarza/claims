use super::super::SCHEMA_VERSION;
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;

pub(super) fn load_and_validate_plan(path: &Path, keep: usize) -> Result<serde_json::Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read survivors plan {}", path.display()))?;
    let plan: serde_json::Value =
        serde_json::from_str(&raw).context("survivors.json is not valid JSON")?;
    validate_plan(&plan, keep)?;
    Ok(plan)
}

fn validate_plan_candidate(idx: usize, c: &serde_json::Value) -> Result<bool> {
    let cid = c["id"]
        .as_str()
        .ok_or_else(|| anyhow!("candidate[{}] missing 'id'", idx))?;
    let keep = c["keep"].as_bool();
    if keep.is_none() {
        bail!("candidate {} missing 'keep' boolean", cid);
    }
    if c["debate"].is_null() {
        bail!(
            "candidate {} has debate=null. archaeology commit requires curated survivors. \
             run the debate phase first (see docs/archaeology.md phase 2).",
            cid
        );
    }
    let verdict = c["debate"]["judge"]["verdict"].as_str();
    if verdict.is_none() {
        bail!(
            "candidate {} missing debate.judge.verdict (must be \"keep\" or \"drop\")",
            cid
        );
    }
    let keeps = keep.unwrap_or(false);
    if keeps && verdict != Some("keep") {
        bail!(
            "candidate {} has keep:true but debate.judge.verdict={:?}",
            cid, verdict
        );
    }
    Ok(keeps)
}

fn validate_plan(plan: &serde_json::Value, keep_cap: usize) -> Result<()> {
    let v = plan["version"].as_str().unwrap_or("");
    if v != SCHEMA_VERSION {
        bail!(
            "survivors.json: expected version '{}', got '{}'",
            SCHEMA_VERSION, v
        );
    }
    let candidates = plan["candidates"]
        .as_array()
        .ok_or_else(|| anyhow!("survivors.json: 'candidates' must be an array"))?;
    let mut keep_count = 0usize;
    for (idx, c) in candidates.iter().enumerate() {
        if validate_plan_candidate(idx, c)? {
            keep_count += 1;
        }
    }
    if keep_count > keep_cap {
        bail!(
            "survivors.json has {} keep:true rows, exceeds --keep {}. tighten the judge or raise the cap",
            keep_count, keep_cap
        );
    }
    Ok(())
}

