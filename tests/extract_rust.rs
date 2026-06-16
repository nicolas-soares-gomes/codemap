//! Tier0 extraction test for RUST (the "per-language test" pattern — each new language in
//! M5 gets an analogous file).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = r#"
pub mod billing {
    pub struct Invoice {
        pub total: u64,
    }
    impl Invoice {
        pub fn new() -> Self { Invoice { total: 0 } }
        pub fn charge(&self, amount: u64) -> u64 { amount }
    }
    pub enum Status { Open, Paid }
    pub const RATE: u64 = 5;
    pub fn helper() {}
}
pub trait Payable {
    fn pay(&self);
}
"#;

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_rust_symbols_with_name_paths() {
    let syms = ts::extract(Language::Rust, SRC);

    assert!(has(&syms, "billing", SymbolKind::Module));
    assert!(has(&syms, "billing/Invoice", SymbolKind::Struct));
    assert!(has(&syms, "billing/Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "billing/Invoice/new", SymbolKind::Method));
    assert!(has(&syms, "billing/Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "billing/Status", SymbolKind::Enum));
    assert!(has(&syms, "billing/Status/Open", SymbolKind::Variant));
    assert!(has(&syms, "billing/Status/Paid", SymbolKind::Variant));
    assert!(has(&syms, "billing/RATE", SymbolKind::Const));
    assert!(has(&syms, "billing/helper", SymbolKind::Function));
    assert!(has(&syms, "Payable", SymbolKind::Trait));
    assert!(has(&syms, "Payable/pay", SymbolKind::Method));
}

#[test]
fn ranges_are_one_based_and_sane() {
    let syms = ts::extract(Language::Rust, SRC);
    let charge = syms
        .iter()
        .find(|s| s.name_path == "billing/Invoice/charge")
        .expect("charge present");
    assert!(charge.range.start_line >= 1);
    assert!(charge.range.end_line >= charge.range.start_line);
    assert!(charge.sel_col > 0);
}

#[test]
fn unsupported_language_returns_empty() {
    // Clojure has no compatible tree-sitter grammar yet (pins an older tree-sitter crate).
    assert!(ts::extract(Language::Clojure, "(defn main [] 1)\n").is_empty());
}
