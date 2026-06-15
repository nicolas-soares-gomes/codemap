//! `codemap doctor` — detect-only diagnostics; never installs anything. M0 stub; the
//! per-language SCIP/LSP detection matrix lands in M3.

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("codemap doctor");
    println!("  schema_version : {}", crate::db::SCHEMA_VERSION);
    println!("  policy         : never installs LSP/SCIP — detects and instructs only (M3)");
    println!("  tier0          : tree-sitter (M1)");
    Ok(())
}
