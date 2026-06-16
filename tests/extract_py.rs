//! Tier0 extraction test for PYTHON (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
class PaymentService:
    def charge(self, amount):
        return amount

def helper():
    pass
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_python_symbols_with_name_paths() {
    let syms = ts::extract(Language::Python, SRC);
    assert!(has(&syms, "PaymentService", SymbolKind::Class));
    assert!(has(&syms, "PaymentService/charge", SymbolKind::Method));
    assert!(has(&syms, "helper", SymbolKind::Function));
}

#[test]
fn extracts_python_calls() {
    let src = "def callee():\n    pass\ndef caller():\n    callee()\n";
    let calls = ts::extract_calls(Language::Python, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
