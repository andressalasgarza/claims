use anyhow::Result;

use crate::commands::util::refuse_in_repair_mode;
use crate::output::OutputFormat;
use crate::store::Store;

pub(crate) fn cmd_migrate_integrity(store: &mut Store, fmt: OutputFormat) -> Result<()> {
    refuse_in_repair_mode("migrate-integrity")?;
    let outcome = store.migrate_integrity()?;
    if matches!(fmt, OutputFormat::Ai) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "checked": outcome.checked,
                "upgraded": outcome.upgraded,
            }))?
        );
        return Ok(());
    }
    println!(
        "migrate-integrity: checked={} upgraded={} mandatory integrity_mac is now enforced",
        outcome.checked, outcome.upgraded,
    );
    Ok(())
}
