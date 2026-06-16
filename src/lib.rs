//! codemap — deterministic code index for AI agents.

#![allow(dead_code)]

pub mod db;
pub mod doctor;
pub mod export;
pub mod index;
#[cfg(feature = "tier2-lsp")]
pub mod lsp;
pub mod query;
pub mod scip;
pub mod skills;
pub mod ts;
pub mod types;
