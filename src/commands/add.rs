use anyhow::{anyhow, Result};
use chrono::Utc;
use ulid::Ulid;

use crate::cli::AddArgs;
use crate::commands::util::refuse_in_repair_mode;
use crate::git;
use crate::models::{Claim, ConfidenceTier, Edge, EdgeType, State, SCHEMA_VERSION};
use crate::output::{self, OutputFormat};
use crate::store::Store;

fn build_add_edges(depends_on: &[u64], tests: &[u64]) -> Vec<Edge> {
    let mut edges = Vec::with_capacity(depends_on.len() + tests.len());
    for d in depends_on {
        edges.push(Edge {
            r#type: EdgeType::DependsOn,
            to_seq: *d,
        });
    }
    for t in tests {
        edges.push(Edge {
            r#type: EdgeType::Tests,
            to_seq: *t,
        });
    }
    edges
}

fn parse_created_at(s: &str) -> Result<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow!("--created-at must be RFC3339 (e.g. 2024-01-15T12:34:56Z): {}", e))
}

/// reject empty text and dangling --depends-on / --tests references.
/// extracted so cmd_add stays a flat sequence of phase calls.
fn validate_add_inputs(store: &Store, a: &AddArgs) -> Result<()> {
    if a.text.trim().is_empty() {
        return Err(anyhow!("claim text cannot be empty"));
    }
    for d in a.depends_on.iter().chain(a.tests.iter()) {
        if store.read_claim(*d).is_err() {
            return Err(anyhow!("referenced claim #{} does not exist", d));
        }
    }
    Ok(())
}

/// backfill gate: --created-at and --git-sha forge provenance metadata.
/// they are documented for archaeology workflows but were exposed on every
/// `clms add`, allowing any agent to stamp a claim with arbitrary git_sha
/// and created_at (e.g., year 9999, deadbeef..., a colleague's actual sha).
/// require CLAIMS_BACKFILL=1 to use them. archaeology constructs claims
/// directly via the Claim struct, not via cmd_add, so it is unaffected.
fn assert_backfill_authorized(a: &AddArgs) -> Result<()> {
    let has_override = a.created_at_override.is_some() || a.git_sha_override.is_some();
    let authorized = std::env::var("CLAIMS_BACKFILL").is_ok();
    if !has_override || authorized {
        return Ok(());
    }
    let mut flags = Vec::new();
    if a.created_at_override.is_some() {
        flags.push("--created-at");
    }
    if a.git_sha_override.is_some() {
        flags.push("--git-sha");
    }
    Err(anyhow!(
        "refusing to backfill provenance: {} requires CLAIMS_BACKFILL=1.\n\nthese flags forge git_sha and created_at; they are intended for archaeology / historical reconstruction only. set CLAIMS_BACKFILL=1 in the environment to authorize, then re-run.",
        flags.join(" + "),
    ))
}

/// compute (created_at, git_sha, backfilled). bundled because the three are
/// derived from the same two override flags. backfilled=true tracks whether
/// EITHER override was used so future readers know the provenance is
/// retroactively asserted rather than captured at write time.
fn resolve_provenance(
    a: &AddArgs,
) -> Result<(chrono::DateTime<Utc>, Option<String>, bool)> {
    let stamp_at = match a.created_at_override.as_deref() {
        Some(s) => parse_created_at(s)?,
        None => Utc::now(),
    };
    let stamp_sha = a.git_sha_override.clone().or_else(git::current_sha);
    let backfilled = a.created_at_override.is_some() || a.git_sha_override.is_some();
    Ok((stamp_at, stamp_sha, backfilled))
}

/// build the initial Claim record from validated inputs. all fields the
/// store later populates (content_hash, integrity_mac) start None.
fn build_initial_claim(
    store: &mut Store, a: AddArgs, stamp_at: chrono::DateTime<Utc>,
    stamp_sha: Option<String>, backfilled: bool, min_tier: Option<ConfidenceTier>,
) -> Result<Claim> {
    let state = if a.unverifiable { State::Unverifiable } else { State::Pending };
    Ok(Claim {
        schema_version: SCHEMA_VERSION.into(),
        id: Ulid::new(),
        seq: store.next_seq()?,
        text: a.text,
        state,
        tags: a.tag,
        evidence: vec![],
        edges: build_add_edges(&a.depends_on, &a.tests),
        agent: a.agent,
        session: a.session,
        git_sha: stamp_sha,
        created_at: stamp_at,
        updated_at: stamp_at,
        content_hash: None,
        integrity_mac: None,
        archaeology_meta: None,
        backfilled,
        min_tier,
    })
}

pub(crate) fn cmd_add(store: &mut Store, a: AddArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("add")?;
    validate_add_inputs(store, &a)?;
    assert_backfill_authorized(&a)?;
    let (stamp_at, stamp_sha, backfilled) = resolve_provenance(&a)?;
    let min_tier = a.min_tier.as_deref().map(ConfidenceTier::parse).transpose()?;
    let mut claim = build_initial_claim(store, a, stamp_at, stamp_sha, backfilled, min_tier)?;
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}
