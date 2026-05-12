use crate::models::Evidence;
use anyhow::{anyhow, Result};

pub(super) fn missing_ref(ev: &Evidence) -> bool {
    ev.r#ref.is_empty()
}

#[inline]
pub(super) fn missing_str(opt: &Option<String>) -> bool {
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
pub(super) fn req_artifact_inputs(
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
pub(super) fn req_finite_field(
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
pub(super) fn req_u64_at_least(
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
pub(super) fn req_present<T>(opt: Option<T>, err: impl FnOnce() -> anyhow::Error) -> Result<T> {
    opt.ok_or_else(err)
}

