//! Tier0 extraction test for PHP (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = r#"<?php
namespace Billing;

class Invoice {
    public int $total;
    public function charge(int $amount): int { return $amount; }
}

interface Payable {
    public function pay(): void;
}

function helper(): void {}
"#;

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_php_symbols_with_name_paths() {
    let syms = ts::extract(Language::Php, SRC);
    assert!(has(&syms, "Billing", SymbolKind::Module));
    assert!(has(&syms, "Billing/Invoice", SymbolKind::Class));
    assert!(has(&syms, "Billing/Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "Billing/Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "Billing/Payable", SymbolKind::Interface));
    assert!(has(&syms, "Billing/Payable/pay", SymbolKind::Method));
    assert!(has(&syms, "Billing/helper", SymbolKind::Function));
}

#[test]
fn extracts_php_calls() {
    let src = "<?php\nfunction callee() {}\nfunction caller() { callee(); }\n";
    let calls = ts::extract_calls(Language::Php, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
