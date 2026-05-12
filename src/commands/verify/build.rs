//! evidence record construction + cli flag exclusivity gate. lives here
//! because both helpers consume `VerifyArgs` and are only used by
//! `cmd_verify`.

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::cli::VerifyArgs;
use crate::models::{DataSource, Evidence, EvidenceMethod};
use crate::store::hash_ref_if_local;

/// reject cli flags that are exclusive to a single method when used with
/// the wrong method. driven by `MethodSpec::exclusive_flag` so adding a
/// new flag-bearing method requires no changes here.
fn check_exclusive_flags(method: EvidenceMethod, a: &VerifyArgs) -> Result<()> {
    use crate::models::METHODS;
    // single-owner flags: exactly one method owns each. driven by
    // MethodSpec::exclusive_flag so adding a new flag-bearing method needs
    // no changes here.
    let single_owner_flags: [(&str, bool); 4] = [
        ("target", a.target.is_some()),
        ("dataset", a.dataset.is_some()),
        ("metric", a.metric.is_some()),
        ("estimator", a.estimator.is_some()),
    ];
    for (flag_name, is_present) in single_owner_flags {
        if !is_present {
            continue;
        }
        let owner = METHODS.iter().find(|m| m.exclusive_flag == Some(flag_name));
        match owner {
            Some(spec) if spec.variant == method => {} // ok: method owns this flag
            Some(spec) => {
                return Err(anyhow!(
                    "--{} is only valid with --method {} (got --method {})",
                    flag_name,
                    spec.name,
                    method.as_str()
                ));
            }
            None => {} // flag has no exclusivity constraint
        }
    }
    // data-source is shared by stat-test (real|live samples) and benchmark
    // (eval set provenance). refuse it on every other method. cannot use
    // the single-owner mechanism above since two methods legitimately
    // accept it.
    if a.data_source.is_some()
        && !matches!(
            method,
            EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate
        )
    {
        return Err(anyhow!(
            "--data-source is only valid with --method stat-test, --method benchmark, or --method estimate (got --method {})",
            method.as_str()
        ));
    }
    Ok(())
}

pub(crate) fn build_evidence(a: &VerifyArgs, m: EvidenceMethod) -> Result<Evidence> {
    check_exclusive_flags(m, a)?;
    // data-source is parsed for stat-test, benchmark, and estimate (the
    // three methods that legitimately accept it). check_exclusive_flags has
    // already refused it on every other method, so a non-None data-source
    // here is guaranteed to belong to one of those three.
    let data_source = match (&m, &a.data_source) {
        (
            EvidenceMethod::StatTest | EvidenceMethod::Benchmark | EvidenceMethod::Estimate,
            Some(s),
        ) => Some(DataSource::parse(s)?),
        _ => None,
    };
    let dataset_hash = a.dataset.as_deref().and_then(hash_ref_if_local);
    let cmd_hash = a
        .cmd
        .as_deref()
        .map(|c| blake3::hash(c.as_bytes()).to_hex().to_string());
    Ok(Evidence {
        method: m,
        r#ref: a.r#ref.clone(),
        note: a.note.clone(),
        p_value: a.p_value,
        sample_size: a.sample_size,
        test_type: a.test_type,
        exit_code: a.exit_code,
        quote: a.quote.clone(),
        from_claims: a.from.clone(),
        ref_hash: hash_ref_if_local(&a.r#ref),
        cmd: a.cmd.clone(),
        cmd_hash,
        stdout_hash: None,
        target: a.target.clone(),
        dataset: a.dataset.clone(),
        dataset_hash,
        data_source,
        metric: a.metric,
        metric_value: a.metric_value,
        threshold: a.threshold,
        estimator: a.estimator,
        point_value: a.point_value,
        ci_lower: a.ci_lower,
        ci_upper: a.ci_upper,
        confidence_level: a.confidence_level,
        recorded_at: Utc::now(),
    })
}
