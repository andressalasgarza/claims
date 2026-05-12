use super::Store;
use crate::models::{Evidence, State};
use anyhow::{anyhow, Result};

/// derived parent validation. before this commit, the only check was
/// `from_claims.len() >= 2`. that left several trivial bypasses in production:
/// same id twice, non-existent parents, unverified parents (PENDING),
/// refuted parents, and self-derivation (cite yourself to verify yourself).
/// all now refuse with specific messages. derived must compose only over
/// claims that have actually crossed the falsifiability bar.
/// derived-input gates: arity, self-reference, dedup. flat sequence so a
/// future maintainer adding another "input shape" check has one obvious
/// home. saves 3 ccn from the parent.
fn derived_check_inputs(ev: &Evidence, current_seq: u64) -> Result<()> {
    if ev.from_claims.len() < 2 {
        return Err(anyhow!(
            "derived requires at least two --from <claim_id> entries"
        ));
    }
    if ev.from_claims.contains(&current_seq) {
        return Err(anyhow!(
            "derived: claim #{} cannot cite itself in --from. self-derivation is circular.",
            current_seq
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for id in &ev.from_claims {
        if !seen.insert(*id) {
            return Err(anyhow!(
                "derived: --from #{} listed twice. each parent claim must be unique.",
                id
            ));
        }
    }
    Ok(())
}

/// every parent must exist and be in Verified state. extracted so the
/// parent function carries one decision (the for loop) instead of three.
fn derived_check_parents(store: &Store, ev: &Evidence) -> Result<()> {
    for id in &ev.from_claims {
        let parent = store.read_claim(*id).map_err(|_| {
            anyhow!(
                "derived: --from #{} does not exist. cannot derive from a non-existent claim.",
                id
            )
        })?;
        if !matches!(parent.state, State::Verified) {
            return Err(anyhow!(
                "derived: --from #{} is {} (not verified). derived claims must compose only over verified parents — deriving from {} is unsound.",
                id,
                parent.state.as_str(),
                parent.state.as_str()
            ));
        }
    }
    Ok(())
}

/// cycle detection: walking from each --from parent, the current claim must
/// not be reachable through any chain of derived edges. without this, X can
/// derive from {Y, Z} while Y derives from {X, Z} — both verify, and the
/// "derived" relation is no longer a DAG. the falsifiability story collapses
/// because each claim's evidence ultimately rests on its own conclusion.
fn derived_check_dag(store: &Store, ev: &Evidence, current_seq: u64) -> Result<()> {
    let mut visited = std::collections::HashSet::new();
    for parent_id in &ev.from_claims {
        if let Some(cycle_path) = derived_cycle_path(store, *parent_id, current_seq, &mut visited)? {
            // cycle_path is [parent, ..., target]; prepend current_seq for the
            // human-readable round trip current_seq → parent → … → current_seq.
            let path_str = std::iter::once(current_seq)
                .chain(cycle_path.into_iter())
                .map(|s| format!("#{}", s))
                .collect::<Vec<_>>()
                .join(" → ");
            return Err(anyhow!(
                "derived: cycle detected. claim #{} would derive from #{}, but #{} is already (transitively) derived from #{}.\n  cycle: {}\n\nderived edges must form a DAG. break the cycle by refuting one of the participants or by re-grounding it in non-derived evidence.",
                current_seq, parent_id, parent_id, current_seq,
                path_str,
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_derived(ev: &Evidence, store: &Store, current_seq: u64) -> Result<()> {
    derived_check_inputs(ev, current_seq)?;
    derived_check_parents(store, ev)?;
    derived_check_dag(store, ev, current_seq)?;
    Ok(())
}

/// DFS from `start` looking for `target`. returns Some(path-from-start-to-target)
/// if a cycle exists, None otherwise. `visited` is shared across calls so we
/// only walk each parent once per validate_derived call (linear in graph size).
fn derived_cycle_path(
    store: &Store,
    start: u64,
    target: u64,
    visited: &mut std::collections::HashSet<u64>,
) -> Result<Option<Vec<u64>>> {
    if start == target {
        return Ok(Some(vec![start]));
    }
    if !visited.insert(start) {
        return Ok(None);
    }
    let claim = match store.read_claim(start) {
        Ok(c) => c,
        Err(_) => return Ok(None), // missing parent already rejected upstream
    };
    for ev in &claim.evidence {
        if !matches!(ev.method, crate::models::EvidenceMethod::Derived) {
            continue;
        }
        for next in &ev.from_claims {
            if let Some(mut sub) = derived_cycle_path(store, *next, target, visited)? {
                sub.insert(0, start);
                return Ok(Some(sub));
            }
        }
    }
    Ok(None)
}

