use super::common::missing_ref;
use super::reference::ref_shape_acceptable;
use crate::models::Evidence;
use anyhow::{anyhow, Result};

pub(super) fn validate_observed(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("observed requires --ref <path-url-or-hash>"));
    }
    if !ref_shape_acceptable(&ev.r#ref) {
        return Err(anyhow!(
            "observed --ref '{}' is not auditable. it must be one of: (a) an existing local file path, (b) a URL (http/https/file/ftp/ssh/git/ws), or (c) a content-address (sha256:HEX / blake3:HEX / sha1:HEX). bare strings are refused because they provide no surface for refutation.",
            ev.r#ref
        ));
    }
    Ok(())
}

