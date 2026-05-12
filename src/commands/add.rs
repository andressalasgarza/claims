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

pub(crate) fn cmd_add(store: &mut Store, a: AddArgs, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("add")?;
    if a.text.trim().is_empty() {
        return Err(anyhow!("claim text cannot be empty"));
    }
    for d in a.depends_on.iter().chain(a.tests.iter()) {
        if store.read_claim(*d).is_err() {
            return Err(anyhow!("referenced claim #{} does not exist", d));
        }
    }
    // backfill gate: --created-at and --git-sha forge provenance metadata.
    // they are documented for archaeology workflows but were exposed on every
    // `clms add`, allowing any agent to stamp a claim with arbitrary git_sha
    // and created_at (e.g., year 9999, deadbeef..., a colleague's actual sha).
    // require CLAIMS_BACKFILL=1 to use them. archaeology constructs claims
    // directly via the Claim struct, not via cmd_add, so it is unaffected.
    let backfill_authorized = std::env::var("CLAIMS_BACKFILL").is_ok();
    if (a.created_at_override.is_some() || a.git_sha_override.is_some()) && !backfill_authorized {
        let mut flags = Vec::new();
        if a.created_at_override.is_some() {
            flags.push("--created-at");
        }
        if a.git_sha_override.is_some() {
            flags.push("--git-sha");
        }
        return Err(anyhow!(
            "refusing to backfill provenance: {} requires CLAIMS_BACKFILL=1.\n\nthese flags forge git_sha and created_at; they are intended for archaeology / historical reconstruction only. set CLAIMS_BACKFILL=1 in the environment to authorize, then re-run.",
            flags.join(" + "),
        ));
    }
    let stamp_at = match a.created_at_override.as_deref() {
        Some(s) => parse_created_at(s)?,
        None => Utc::now(),
    };
    let stamp_sha = a.git_sha_override.clone().or_else(git::current_sha);
    // stamp backfilled=true whenever provenance overrides were actually used
    // (presence-checked above; CLAIMS_BACKFILL only authorizes them, doesn't
    // mark the record). future-me reading this claim can then see "this
    // git_sha and created_at were retroactively asserted, not auto-captured."
    let backfilled = a.created_at_override.is_some() || a.git_sha_override.is_some();
    let min_tier = a.min_tier.as_deref().map(ConfidenceTier::parse).transpose()?;
    let mut claim = Claim {
        schema_version: SCHEMA_VERSION.into(),
        id: Ulid::new(),
        seq: store.next_seq()?,
        text: a.text,
        state: if a.unverifiable {
            State::Unverifiable
        } else {
            State::Pending
        },
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
    };
    store.write_claim(&mut claim)?;
    print!("{}", output::render_claim(&claim, store, fmt)?);
    Ok(())
}
