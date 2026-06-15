//! Tier0 extraction test for TYPESCRIPT (per-language test pattern).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = r#"
export namespace billing {
  export class Invoice {
    total: number = 0;
    charge(amount: number): number { return amount; }
  }
  export interface Payable {
    pay(): void;
  }
  export enum Status { Open, Paid }
  export function helper(): void {}
  export const compute = (x: number): number => x * 2;
}
"#;

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter().any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_typescript_symbols_with_name_paths() {
    let syms = ts::extract(Language::TypeScript, SRC);

    assert!(has(&syms, "billing", SymbolKind::Module));
    assert!(has(&syms, "billing/Invoice", SymbolKind::Class));
    assert!(has(&syms, "billing/Invoice/total", SymbolKind::Field));
    assert!(has(&syms, "billing/Invoice/charge", SymbolKind::Method));
    assert!(has(&syms, "billing/Payable", SymbolKind::Interface));
    assert!(has(&syms, "billing/Payable/pay", SymbolKind::Method));
    assert!(has(&syms, "billing/Status", SymbolKind::Enum));
    assert!(has(&syms, "billing/Status/Open", SymbolKind::Variant));
    assert!(has(&syms, "billing/Status/Paid", SymbolKind::Variant));
    assert!(has(&syms, "billing/helper", SymbolKind::Function));
    assert!(has(&syms, "billing/compute", SymbolKind::Function), "const arrow fn");
}

#[test]
fn extracts_typescript_calls() {
    let src = "function callee(): void {}\nfunction caller(): void { callee(); }\n";
    let calls = ts::extract_calls(Language::TypeScript, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
}
