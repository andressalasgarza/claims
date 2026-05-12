use super::derived::validate_derived;
use super::Store;
use crate::models::Evidence;
use anyhow::{anyhow, Result};

mod artifact;

use artifact::{validate_benchmark, validate_estimate, validate_stat_test};

/// dispatch to per-method validator. behavior preserved exactly; the only
/// change vs pre-refactor is that each per-method block is now its own fn,
/// so cyclomatic complexity is bounded per validator instead of stacking.
pub fn validate_evidence(ev: &Evidence, store: &Store, current_seq: u64) -> Result<()> {
    use crate::models::EvidenceMethod::*;
    match ev.method {
        PropTest => validate_prop_test(ev),
        IntegrationTest => validate_integration_test(ev),
        ReplayTest => validate_replay_test(ev),
        StatTest => validate_stat_test(ev),
        Benchmark => validate_benchmark(ev),
        Estimate => validate_estimate(ev),
        Observed => validate_observed(ev),
        Documented => validate_documented(ev),
        Derived => validate_derived(ev, store, current_seq),
    }
}
fn missing_ref(ev: &Evidence) -> bool {
    ev.r#ref.is_empty()
}

#[inline]
fn missing_str(opt: &Option<String>) -> bool {
    opt.as_deref().unwrap_or("").trim().is_empty()
}

// ---------- per-field validation helpers ----------
//
// each helper bundles two or more inline checks (presence + shape) into a
// single call site. drops caller ccn by (n_internal_decisions - 1) per use
// without changing user-visible error wording: the caller still owns the
// error message via closures, so the golden file stays byte-identical.

/// the artifact-driven preamble shared by stat-test/benchmark/estimate:
/// --ref must be present, must hash as a local file, and --cmd must be
/// non-empty. centralizing the trio drops 3 ccn from each validate_* that
/// uses it.
fn req_artifact_inputs(
    ev: &Evidence, missing_ref_msg: &'static str, missing_local_msg: &'static str,
    missing_cmd_msg: &'static str,
) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("{}", missing_ref_msg));
    }
    if ev.ref_hash.is_none() {
        return Err(anyhow!("{}", missing_local_msg));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!("{}", missing_cmd_msg));
    }
    Ok(())
}

/// present-and-finite: bundles `.ok_or_else()? + !is_finite()` into one
/// caller decision. each use saves 1 ccn vs inline.
fn req_finite_field(
    opt: Option<f64>, miss_err: impl FnOnce() -> anyhow::Error,
    nonfinite_err: impl FnOnce(f64) -> anyhow::Error,
) -> Result<f64> {
    let v = opt.ok_or_else(miss_err)?;
    if !v.is_finite() {
        return Err(nonfinite_err(v));
    }
    Ok(v)
}

/// present-and-at-least: bundles `.ok_or_else()? + n < lower` into one
/// caller decision. used for sample_size >= 2 across all 3 artifact methods.
fn req_u64_at_least(
    opt: Option<u64>, lower: u64, miss_err: impl FnOnce() -> anyhow::Error,
    too_low_err: impl FnOnce(u64) -> anyhow::Error,
) -> Result<u64> {
    let n = opt.ok_or_else(miss_err)?;
    if n < lower {
        return Err(too_low_err(n));
    }
    Ok(n)
}

/// some-or-else for non-numeric option fields (estimator, test_type,
/// data_source). just a thin alias to make call sites uniform.
fn req_present<T>(opt: Option<T>, err: impl FnOnce() -> anyhow::Error) -> Result<T> {
    opt.ok_or_else(err)
}

fn validate_prop_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("prop-test requires --ref <path-to-test-file>"));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "prop-test requires --cmd \"<shell cmd that ran the proptest/quickcheck/fuzz>\". the cmd is executed at verify time and the actual exit_code is captured."
        ));
    }
    Ok(())
}

fn validate_integration_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("integration-test requires --ref <path-to-test-file>"));
    }
    if missing_str(&ev.target) {
        return Err(anyhow!(
            "integration-test requires --target <url-or-host>. the test must hit a real external system the author does not control; without --target the falsification surface is undocumented."
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "integration-test requires --cmd \"<shell cmd that probed --target>\"."
        ));
    }
    Ok(())
}

fn validate_replay_test(ev: &Evidence) -> Result<()> {
    if missing_ref(ev) {
        return Err(anyhow!("replay-test requires --ref <path-to-strategy-or-replay-script>"));
    }
    let dataset = ev.dataset.as_deref().unwrap_or("").trim();
    if dataset.is_empty() {
        return Err(anyhow!(
            "replay-test requires --dataset <path>. the dataset must be real-world capture (not synthetic); it is content-hashed for tamper-evidence."
        ));
    }
    if ev.dataset_hash.is_none() {
        return Err(anyhow!(
            "replay-test requires the dataset at --dataset to exist as a local file at verify time so it can be content-hashed. path '{}' could not be hashed.",
            dataset
        ));
    }
    if missing_str(&ev.cmd) {
        return Err(anyhow!(
            "replay-test requires --cmd \"<shell cmd that replayed --dataset through --ref>\"."
        ));
    }
    Ok(())
}

/// observed --ref must be one of:
///   - an existing local file (hashable; the H17 path will content_hash it)
///   - a URL with a recognized scheme (http/https/file/ftp/ssh/git)
///   - a content-address: sha256:HEX / blake3:HEX / sha1:HEX (any hex case)
///
/// random strings like "foo" or "my notes" are refused. without this check
/// observed degrades to "agent typed any string" and provides no auditable
/// surface for refutation.
pub fn ref_shape_acceptable(s: &str) -> bool {
    if std::path::Path::new(s).exists() {
        return true;
    }
    let recognized_url_schemes = [
        "http://", "https://", "file://", "ftp://", "ftps://", "ssh://", "git://", "ws://", "wss://",
    ];
    if recognized_url_schemes.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // content-address: <hashname>:<hex>
    if let Some((scheme, hex)) = s.split_once(':') {
        if matches!(scheme, "sha256" | "sha1" | "sha512" | "blake3" | "md5")
            && !hex.is_empty()
            && hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            return true;
        }
    }
    false
}

fn validate_observed(ev: &Evidence) -> Result<()> {
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

/// is this --target a loopback or RFC1918 / link-local / unspecified address?
/// rejects: localhost, 127/8, 0/8, 10/8, 172.16/12, 192.168/16, 169.254/16,
/// ::1, fc00::/7, fe80::/10. matches by hostname *or* parsed IP literal so
/// both `https://localhost:8080/health` and `http://127.0.0.1` are caught.
/// extract the bare host from a url-shaped target: strip scheme, path,
/// :port suffix, and surrounding [ ] for ipv6 literals.
fn extract_host_from_target(target: &str) -> &str {
    let after_scheme = target.split_once("://").map(|(_, r)| r).unwrap_or(target);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = match host_port.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host_port,
    };
    host.trim_start_matches('[').trim_end_matches(']')
}

/// hard-coded local host literals. matched case-insensitively against the
/// extracted host. centralizing the list keeps target_is_local flat.
const LOCAL_HOST_LITERALS: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "",
    "::",
    "::1",
];

/// ipv4 loopback/private/link-local/unspecified/0.0.0.0-prefix check.
/// extracted so target_is_local stays flat.
fn ipv4_is_local(v4: std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.octets()[0] == 0
}

/// ipv6 loopback/unspecified/unique-local(fc00::/7)/link-local(fe80::/10).
fn ipv6_is_local(v6: std::net::Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    let s = v6.segments()[0];
    (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
}

pub fn target_is_local(target: &str) -> bool {
    let host = extract_host_from_target(target);
    let lowered = host.to_ascii_lowercase();
    if LOCAL_HOST_LITERALS.iter().any(|lit| *lit == lowered) {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => ipv4_is_local(v4),
        Ok(std::net::IpAddr::V6(v6)) => ipv6_is_local(v6),
        Err(_) => false,
    }
}

fn validate_documented(ev: &Evidence) -> Result<()> {
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

