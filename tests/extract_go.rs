//! Symbol extraction test for GO (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
package billing

type Invoice struct {
\tTotal int
}

type Payable interface {
\tPay()
}

func (s *Invoice) Charge(amount int) int { return amount }

func Helper() {}

const Rate = 5
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_go_symbols_with_name_paths() {
    let syms = ts::extract(Language::Go, SRC);
    assert!(has(&syms, "Invoice", SymbolKind::Struct));
    assert!(has(&syms, "Payable", SymbolKind::Interface));
    assert!(
        has(&syms, "Invoice/Charge", SymbolKind::Method),
        "receiver-prefixed method"
    );
    assert!(has(&syms, "Helper", SymbolKind::Function));
    assert!(has(&syms, "Rate", SymbolKind::Const));
}

#[test]
fn extracts_go_calls() {
    let src = "package main\nfunc callee() {}\nfunc caller() { callee() }\n";
    let calls = ts::extract_calls(Language::Go, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
