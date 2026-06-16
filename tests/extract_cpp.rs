//! Tier0 extraction test for C++ (per-language test pattern; shares the C walker).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
namespace billing {
class Invoice {
public:
    int total;
    int charge(int amount) { return amount; }
};
enum class Status { Open, Paid };
}
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_cpp_symbols_with_name_paths() {
    let syms = ts::extract(Language::Cpp, SRC);
    assert!(has(&syms, "billing", SymbolKind::Module));
    assert!(has(&syms, "billing/Invoice", SymbolKind::Class));
    assert!(has(&syms, "billing/Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "billing/Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "billing/Status", SymbolKind::Enum));
    assert!(has(&syms, "billing/Status/Open", SymbolKind::Variant));
}

#[test]
fn extracts_cpp_calls() {
    let src = "void callee() {}\nvoid caller() { callee(); }\n";
    let calls = ts::extract_calls(Language::Cpp, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
