//! `clms schema` command. distinct from `crate::schema` (the data-schema
//! definitions); this one just formats those for the cli surface.

use anyhow::{anyhow, Result};

use crate::output::OutputFormat;
use crate::schema::{self, schema_value};

pub(crate) fn cmd_schema(target: Option<String>, fmt: OutputFormat) -> Result<()> {
    if let Some(t) = target.as_deref() {
        match t {
            "methods" => {
                println!("{}", serde_json::to_string_pretty(&schema::methods_table())?);
                return Ok(());
            }
            other => {
                return Err(anyhow!(
                    "unknown schema target '{}'. valid: methods",
                    other
                ));
            }
        }
    }
    if matches!(fmt, OutputFormat::Ai) {
        println!("{}", serde_json::to_string(&schema_value())?);
        return Ok(());
    }
    let s = schema_value();
    println!("clms schema v{}", s["version"].as_str().unwrap_or("?"));
    println!();
    println!("states:           {}", s["states"]);
    println!("confidence tiers: derived < documented < observed < empirical");
    println!("edge types:       {}", s["edge_types"]);
    println!("output formats:   {}", s["output_formats"]);
    println!();
    println!("evidence method requirements:");
    if let Some(map) = s["evidence_methods"].as_object() {
        for (name, spec) in map {
            let req = spec["required"].to_string();
            let conf = spec["confidence"].as_str().unwrap_or("?");
            println!("  {:<11} required={} → {}", name, req, conf);
        }
    }
    println!();
    if let Some(map) = s["env_vars"].as_object() {
        let names: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
        println!("env vars: {}", names.join(", "));
    }
    println!("under --format ai: errors emit json envelope on stderr (run `clms --format ai schema` for full spec)");
    Ok(())
}
