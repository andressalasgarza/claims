use anyhow::{anyhow, Result};
use crate::models::EvidenceMethod;
use std::path::Path;

/// json field extraction trait. one impl per leaf type we read from
/// artifact files; keeps `artifact_req` / `match_eq` generic without
/// hand-rolling three copies of each.
pub(crate) trait ArtifactField: Sized + PartialEq + std::fmt::Display {
    /// human-readable label for the missing-field error string. preserved
    /// verbatim across the original artifact_req_{str,f64,u64} trio so the
    /// golden error tests keep matching.
    const KIND: &'static str;
    fn extract(v: &serde_json::Value) -> Option<Self>;
}

impl ArtifactField for String {
    const KIND: &'static str = "string";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_str().map(str::to_string) }
}

impl ArtifactField for f64 {
    const KIND: &'static str = "numeric";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_f64() }
}

impl ArtifactField for u64 {
    const KIND: &'static str = "integer";
    fn extract(v: &serde_json::Value) -> Option<Self> { v.as_u64() }
}

pub(crate) fn artifact_req<T: ArtifactField>(
    value: &serde_json::Value,
    method: EvidenceMethod,
    r#ref: &str,
    key: &str,
) -> Result<T> {
    T::extract(&value[key]).ok_or_else(|| {
        anyhow!(
            "{} artifact '{}' is missing {} field '{}'.",
            method.as_str(),
            r#ref,
            T::KIND,
            key,
        )
    })
}

/// echo-check: if the cli supplied a value, refuse on mismatch with the
/// artifact. the artifact is always the source of truth.
pub(crate) fn match_eq<T: ArtifactField>(
    method: EvidenceMethod,
    r#ref: &str,
    flag: &str,
    cli: Option<T>,
    artifact: &T,
) -> Result<()> {
    if let Some(cli) = cli {
        if &cli != artifact {
            return Err(anyhow!(
                "{} artifact '{}' disagrees with --{}: cli gave '{}', artifact says '{}'. clms now treats the artifact as the source of truth; fix the script or drop the stale flag.",
                method.as_str(),
                r#ref,
                flag,
                cli,
                artifact,
            ));
        }
    }
    Ok(())
}

pub(crate) fn read_local_json_artifact(method: EvidenceMethod, r#ref: &str) -> Result<serde_json::Value> {
    let path = Path::new(r#ref);
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

