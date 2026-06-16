//! Tier0 extraction test for SWIFT (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
class Invoice {
    var total = 0
    func charge(a: Int) -> Int { return a }
}
protocol Payable {
    func pay()
}
enum Status { case open
    case paid }
func helper() {}
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_swift_symbols_with_name_paths() {
    let syms = ts::extract(Language::Swift, SRC);
    assert!(has(&syms, "Invoice", SymbolKind::Class));
    assert!(has(&syms, "Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "Payable", SymbolKind::Interface));
    assert!(has(&syms, "Payable/pay", SymbolKind::Method));
    assert!(has(&syms, "Status", SymbolKind::Enum));
    assert!(has(&syms, "Status/open", SymbolKind::Variant));
    assert!(has(&syms, "helper", SymbolKind::Function));
}

#[test]
fn extracts_swift_calls() {
    let src = "func callee() {}\nfunc caller() { callee() }\n";
    let calls = ts::extract_calls(Language::Swift, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
