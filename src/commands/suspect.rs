use anyhow::Result;

use crate::models::{Claim, State};
use crate::output::{self, OutputFormat};
use crate::store::Store;

pub(crate) fn cmd_suspect(store: &Store, fmt: OutputFormat, exclude_agent: &[String]) -> Result<()> {
    let claims: Vec<Claim> = store
        .all_claims()?
        .into_iter()
        .filter(|c| c.state == State::Suspect)
        .filter(|c| !output::agent_excluded(c, exclude_agent))
        .collect();
    if matches!(fmt, OutputFormat::Ai) {
        let arr: Vec<_> = claims
            .iter()
            .map(|c| {
                serde_json::json!({
                    "seq": c.seq,
                    "text": c.text,
                    "tags": c.tags,
                })
            })
            .collect();
        println!("{}", serde_json::to_string(&arr)?);
        return Ok(());
    }
    if claims.is_empty() {
        println!("no suspect claims.");
        return Ok(());
    }
    println!("suspect claims ({}):", claims.len());
    for c in claims {
        println!("  #{} {}", c.seq, c.text);
    }
    Ok(())
}
