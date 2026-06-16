//! Tier0 extraction test for C# (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = r#"
namespace Billing {
    public class Invoice {
        public int Total { get; set; }
        public int Charge(int amount) { return amount; }
    }
    public interface IPayable {
        void Pay();
    }
    public enum Status { Open, Paid }
}
"#;

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_csharp_symbols_with_name_paths() {
    let syms = ts::extract(Language::CSharp, SRC);
    assert!(has(&syms, "Billing", SymbolKind::Module));
    assert!(has(&syms, "Billing/Invoice", SymbolKind::Class));
    assert!(has(&syms, "Billing/Invoice/Charge", SymbolKind::Method));
    assert!(has(&syms, "Billing/Invoice/Total", SymbolKind::Field));
    assert!(has(&syms, "Billing/IPayable", SymbolKind::Interface));
    assert!(has(&syms, "Billing/IPayable/Pay", SymbolKind::Method));
    assert!(has(&syms, "Billing/Status", SymbolKind::Enum));
    assert!(has(&syms, "Billing/Status/Open", SymbolKind::Variant));
}

#[test]
fn extracts_csharp_calls() {
    let src = "class C { void Callee() {} void Caller() { Callee(); } }\n";
    let calls = ts::extract_calls(Language::CSharp, src);
    assert!(calls.iter().any(|c| c.name == "Callee"));
}
