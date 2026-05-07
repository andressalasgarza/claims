use chrono::{DateTime, TimeZone, Utc};
use std::path::Path;
use std::process::Command;

pub fn current_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// author-time of the commit that last touched the given line, via
/// `git blame -L <line>,+1 --porcelain <path>`.
///
/// returns None when:
/// - not in a git repo
/// - file is untracked / .gitignored
/// - line is uncommitted (blame's all-zeros sha; we fall through)
/// - any subprocess failure
pub fn blame_line_time(path: &Path, line: u32) -> Option<DateTime<Utc>> {
    let range = format!("{},+1", line);
    let out = Command::new("git")
        .args(["blame", "-L", &range, "--porcelain"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8(out.stdout).ok()?;
    // first line of porcelain output is `<sha> <orig-line> <final-line> <num-lines>`.
    // sha all zeros means the line is uncommitted (working-tree edit).
    let first = stdout.lines().next()?;
    let sha = first.split_whitespace().next()?;
    if sha.chars().all(|c| c == '0') {
        return None;
    }
    for ln in stdout.lines() {
        if let Some(rest) = ln.strip_prefix("author-time ") {
            let secs: i64 = rest.trim().parse().ok()?;
            return Utc.timestamp_opt(secs, 0).single();
        }
    }
    None
}

/// mtime of a path as a UTC timestamp. used as a fallback when git can't
/// date a candidate (uncommitted annotation, sidecar manifest, etc.).
pub fn file_mtime(path: &Path) -> Option<DateTime<Utc>> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Utc.timestamp_opt(dur.as_secs() as i64, 0).single()
}
