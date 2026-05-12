use super::common::{missing_ref, missing_str};
use crate::models::Evidence;
use anyhow::{anyhow, Result};

pub(super) fn validate_documented(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("documented requires --ref <url>"));
    }
    if missing_str(&ev.quote) {
        return Err(anyhow!(
            "documented requires --quote \"<exact text from doc>\""
        ));
    }
    Ok(())
}

