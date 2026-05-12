use super::key::load_integrity_key;
use super::strict::integrity_strict_mode;
use super::{canonical_content_hash, canonical_integrity_mac};
use crate::models::Claim;
use anyhow::{anyhow, Result};
use std::path::Path;

#[inline]
// ---------- read_claim integrity gates ----------
//
// each `check_*` is one of three states:
//   Ok(true)  – gate passed cleanly; proceed.
//   Ok(false) – gate failed but CLAIMS_REPAIR=1 was set; caller emits the
//                claim as-is for forensic read.
//   Err(_)    – gate failed and repair is off; bubble to the caller.
// splitting this way drops read_claim from a 17-ccn monolith to a flat 3
// linker.

pub(crate) fn check_seq_matches(c: &Claim, seq: u64, path: &Path, repair: bool) -> Result<()> {
    if c.seq == seq || repair {
        return Ok(());
    }
    Err(anyhow!(
        "claim file {} declares seq={} but was loaded as #{}. filename and payload must agree.",
        path.display(), c.seq, seq,
    ))
}

pub(crate) fn check_content_hash(c: &Claim, seq: u64, path: &Path, repair: bool) -> Result<()> {
    let stored = match c.content_hash.as_deref() {
        Some(s) => s,
        None if repair => return Ok(()),
        None => return Err(anyhow!(
            "claim #{} at {} has no content_hash. either it was hand-written outside `clms`, \
or it predates the integrity field. set CLAIMS_REPAIR=1 to read anyway, then re-save with `clms reindex`.",
            seq, path.display(),
        )),
    };
    let recomputed = canonical_content_hash(c);
    if stored == recomputed || repair { return Ok(()); }
    Err(anyhow!(
        "claim #{} content_hash mismatch — file has been tampered.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\nthe json on disk does not match the hash it claims. someone edited the file outside `clms`. \
set CLAIMS_REPAIR=1 to read anyway (use only if you accept the tampered content), then re-save.",
        seq,
        &stored[..16.min(stored.len())],
        &recomputed[..16],
        path.display(),
    ))
}

/// fetch the integrity key or build a stable error message that mentions
/// the underlying failure reason. extracted so check_integrity_mac stays
/// flat.
fn integrity_key_or_err(seq: u64, path: &Path) -> Result<[u8; 32]> {
    load_integrity_key().map_err(|err| anyhow!(
        "claim #{} at {} carries integrity_mac, but the verification key is unavailable: {}. set CLAIMS_REPAIR=1 for forensic read-only access, or restore the key before using this ledger.",
        seq, path.display(), err,
    ))
}

pub(crate) fn check_integrity_mac(c: &Claim, seq: u64, path: &Path, root: &Path, repair: bool) -> Result<()> {
    let stored = match c.integrity_mac.as_deref() {
        Some(s) => s,
        None => return check_integrity_mac_missing(seq, path, root, repair),
    };
    let key = match integrity_key_or_err(seq, path) {
        Ok(k) => k,
        Err(_) if repair => return Ok(()),
        Err(e) => return Err(e),
    };
    let recomputed = canonical_integrity_mac(c, &key);
    if stored == recomputed || repair { return Ok(()); }
    Err(anyhow!(
        "claim #{} integrity_mac mismatch — file has been tampered or the wrong integrity key is in use.\n  stored:     {}\n  recomputed: {}\n  path:       {}\n\ncontent_hash alone is forgeable. integrity_mac is the authoritative check. set CLAIMS_REPAIR=1 only for forensic read-only recovery.",
        seq,
        &stored[..16.min(stored.len())],
        &recomputed[..16],
        path.display(),
    ))
}

/// missing-mac branch: in strict mode every claim must carry a mac. legacy
/// non-strict ledgers tolerate absence; CLAIMS_REPAIR=1 always lets the read
/// proceed for forensic recovery.
fn check_integrity_mac_missing(seq: u64, path: &Path, root: &Path, repair: bool) -> Result<()> {
    if !integrity_strict_mode(root) || repair {
        return Ok(());
    }
    Err(anyhow!(
        "claim #{} at {} has no integrity_mac, but this ledger is in strict integrity mode. migrate legacy hash-only claims with `clms reindex`, or set CLAIMS_REPAIR=1 for forensic read-only access.",
        seq, path.display(),
    ))
}

