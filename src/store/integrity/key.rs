use super::super::{DEFAULT_INTEGRITY_KEY_DIR, DEFAULT_INTEGRITY_KEY_FILE};
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn decode_hex_32(s: &str, label: &str) -> Result<[u8; 32]> {
    let hex = s.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "{} must be exactly 64 hex chars (32 bytes)",
            label
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("{} contains invalid hex", label))?;
    }
    Ok(out)
}

fn default_integrity_key_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        anyhow!(
            "cannot locate HOME for default integrity key path. set CLAIMS_INTEGRITY_KEY_FILE or CLAIMS_INTEGRITY_KEY_HEX."
        )
    })?;
    Ok(PathBuf::from(home)
        .join(DEFAULT_INTEGRITY_KEY_DIR)
        .join(DEFAULT_INTEGRITY_KEY_FILE))
}

fn integrity_key_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CLAIMS_INTEGRITY_KEY_FILE") {
        return Ok(PathBuf::from(path));
    }
    default_integrity_key_path()
}

pub(crate) fn load_integrity_key() -> Result<[u8; 32]> {
    if let Ok(hex) = std::env::var("CLAIMS_INTEGRITY_KEY_HEX") {
        return decode_hex_32(&hex, "CLAIMS_INTEGRITY_KEY_HEX");
    }
    let path = integrity_key_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read integrity key {}", path.display()))?;
    decode_hex_32(&raw, &format!("integrity key file {}", path.display()))
}

/// honor CLAIMS_INTEGRITY_KEY_HEX (test fixture / explicit-key flow):
/// Some(Ok(key)) if env was set + valid; None if env was unset; Err if set
/// but malformed.
fn try_key_from_env() -> Result<Option<[u8; 32]>> {
    let Ok(hex) = std::env::var("CLAIMS_INTEGRITY_KEY_HEX") else {
        return Ok(None);
    };
    Ok(Some(decode_hex_32(&hex, "CLAIMS_INTEGRITY_KEY_HEX")?))
}

/// load from the on-disk key file if present: Some(Ok(key)) if file exists
/// + valid; None if absent; Err if present but unreadable/malformed.
fn try_key_from_file(path: &Path) -> Result<Option<[u8; 32]>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read integrity key {}", path.display()))?;
    Ok(Some(decode_hex_32(&raw, &format!("integrity key file {}", path.display()))?))
}

/// generate a fresh 32-byte key and persist it at `path` with 0o600 perms
/// on unix. caller is responsible for ensuring this is the right moment to
/// mint a new key (i.e., no prior key exists in env or file).
fn generate_and_persist_key(path: &Path) -> Result<[u8; 32]> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create integrity key dir {}", parent.display()))?;
    }
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| anyhow!("generate integrity key: {}", e))?;
    fs::write(path, format!("{}\n", encode_hex(&key)))
        .with_context(|| format!("write integrity key {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {}", path.display()))?;
    }
    Ok(key)
}

pub(crate) fn load_or_create_integrity_key() -> Result<[u8; 32]> {
    if let Some(key) = try_key_from_env()? {
        return Ok(key);
    }
    let path = integrity_key_path()?;
    if let Some(key) = try_key_from_file(&path)? {
        return Ok(key);
    }
    generate_and_persist_key(&path)
}

