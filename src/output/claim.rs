//! Claim rendering across the three output formats (default / human / ai).
//!
//! Split out of output.rs to keep the parent file judged on dispatch and
//! shared helpers only. Format-specific rendering lives in submodules.

mod ai;
mod evidence;
mod human;
mod plain;

pub(super) use ai::render_claim_ai;
pub(super) use human::render_claim_human;
pub(super) use plain::render_claim_default;
