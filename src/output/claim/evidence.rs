use crate::models::{Evidence, EvidenceMethod};

pub(super) fn evidence_extras(e: &Evidence) -> String {
    match e.method {
        EvidenceMethod::StatTest => format!(
            " [{}, p={}, n={}, src={}]",
            e.test_type.map(|t| t.as_str()).unwrap_or("?"),
            e.p_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::PropTest => format!(" [exit={}]", e.exit_code.unwrap_or(-1)),
        EvidenceMethod::IntegrationTest => format!(
            " [exit={}, target={}]",
            e.exit_code.unwrap_or(-1),
            e.target.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::ReplayTest => format!(
            " [exit={}, dataset={}]",
            e.exit_code.unwrap_or(-1),
            e.dataset.as_deref().unwrap_or("?")
        ),
        EvidenceMethod::Benchmark => format!(
            " [{}, value={}, threshold={}, n={}, src={}]",
            e.metric.map(|m| m.as_str()).unwrap_or("?"),
            e.metric_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.threshold.map(|v| v.to_string()).unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Estimate => format!(
            " [{}, point={}, CI=[{}, {}], conf={}, n={}, src={}]",
            e.estimator.map(|m| m.as_str()).unwrap_or("?"),
            e.point_value.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_lower.map(|v| v.to_string()).unwrap_or("?".into()),
            e.ci_upper.map(|v| v.to_string()).unwrap_or("?".into()),
            e.confidence_level
                .map(|v| v.to_string())
                .unwrap_or("?".into()),
            e.sample_size.map(|v| v.to_string()).unwrap_or("?".into()),
            e.data_source.map(|d| d.as_str()).unwrap_or("?")
        ),
        EvidenceMethod::Documented => {
            let q = e.quote.as_deref().unwrap_or("");
            let q = if q.len() > 60 { &q[..60] } else { q };
            format!(" [\"{}\"]", q)
        }
        EvidenceMethod::Derived => format!(" [from {:?}]", e.from_claims),
        _ => String::new(),
    }
}
