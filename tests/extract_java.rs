//! Tier0 extraction test for JAVA (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = r#"
package billing;

public class Invoice {
    private int total;
    public int charge(int amount) { return amount; }
}

interface Payable {
    void pay();
}

enum Status { OPEN, PAID }
"#;

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter().any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_java_symbols_with_name_paths() {
    let syms = ts::extract(Language::Java, SRC);
    assert!(has(&syms, "Invoice", SymbolKind::Class));
    assert!(has(&syms, "Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "Payable", SymbolKind::Interface));
    assert!(has(&syms, "Payable/pay", SymbolKind::Method));
    assert!(has(&syms, "Status", SymbolKind::Enum));
    assert!(has(&syms, "Status/OPEN", SymbolKind::Variant));
}

#[test]
fn extracts_java_calls() {
    let src = "class C {\n  void callee() {}\n  void caller() { callee(); }\n}\n";
    let calls = ts::extract_calls(Language::Java, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
