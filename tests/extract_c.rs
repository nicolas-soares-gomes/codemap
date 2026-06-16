//! Symbol extraction test for C (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
struct Invoice {
    int total;
};

enum Status { OPEN, PAID };

int charge(int amount) {
    return amount;
}

int *helper(void) {
    return 0;
}
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_c_symbols() {
    let syms = ts::extract(Language::C, SRC);
    assert!(has(&syms, "Invoice", SymbolKind::Struct));
    assert!(has(&syms, "Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "Status", SymbolKind::Enum));
    assert!(has(&syms, "Status/OPEN", SymbolKind::Variant));
    assert!(has(&syms, "charge", SymbolKind::Function));
    assert!(
        has(&syms, "helper", SymbolKind::Function),
        "pointer-returning fn name via declarator chain"
    );
}

#[test]
fn extracts_c_calls() {
    let src = "void callee(void) {}\nvoid caller(void) { callee(); }\n";
    let calls = ts::extract_calls(Language::C, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
