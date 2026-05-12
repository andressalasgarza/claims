use super::derived::validate_derived;
use super::Store;
use crate::models::Evidence;
use anyhow::Result;

mod artifact;
mod common;
mod documented;
mod observed;
mod reference;
mod runnable;
mod target;

pub use target::target_is_local;

use artifact::{validate_benchmark, validate_estimate, validate_stat_test};
use documented::validate_documented;
use observed::validate_observed;
use runnable::{validate_integration_test, validate_prop_test, validate_replay_test};

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
