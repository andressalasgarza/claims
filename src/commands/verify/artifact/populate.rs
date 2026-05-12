mod benchmark;
mod estimate;
mod fields;
mod stat_test;

use anyhow::Result;
use crate::models::{Evidence, EvidenceMethod};

use benchmark::populate_benchmark_from_artifact;
use estimate::populate_estimate_from_artifact;
use stat_test::populate_stat_test_from_artifact;

pub(crate) fn populate_empirical_artifact_fields(ev: &mut Evidence) -> Result<()> {
    match ev.method {
        EvidenceMethod::StatTest => populate_stat_test_from_artifact(ev),
        EvidenceMethod::Benchmark => populate_benchmark_from_artifact(ev),
        EvidenceMethod::Estimate => populate_estimate_from_artifact(ev),
        _ => Ok(()),
    }
}
