use anyhow::{anyhow, Result};
use std::path::Path;

pub(crate) fn claim_seq_from_path(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("unexpected claim filename {}", path.display()))?;
    stem.parse::<u64>()
        .map_err(|_| anyhow!("unexpected claim filename {}", path.display()))
}
