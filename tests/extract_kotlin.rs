//! Symbol extraction test for KOTLIN (vendored grammar).

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
package com.example

class Greeter(val name: String) {
    val greeting: String = \"hi\"
    fun greet(): String { return helper() }
    fun helper(): String { return greeting }
}

fun main() {
    val g = Greeter(\"world\")
    g.greet()
}

enum class Color { RED, GREEN }
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_kotlin_symbols_with_name_paths() {
    let syms = ts::extract(Language::Kotlin, SRC);
    assert!(has(&syms, "Greeter", SymbolKind::Class));
    assert!(has(&syms, "Greeter/greet", SymbolKind::Method));
    assert!(has(&syms, "Greeter/helper", SymbolKind::Method));
    assert!(has(&syms, "Greeter/greeting", SymbolKind::Field));
    assert!(has(&syms, "main", SymbolKind::Function));
    assert!(has(&syms, "Color", SymbolKind::Class));
    assert!(has(&syms, "Color/RED", SymbolKind::Variant));
    assert!(has(&syms, "Color/GREEN", SymbolKind::Variant));
}

#[test]
fn extracts_kotlin_calls() {
    let src = "fun callee() {}\nfun caller() { callee() }\n";
    let calls = ts::extract_calls(Language::Kotlin, src);
    assert!(calls.iter().any(|c| c.name == "callee"));

    // Member calls resolve to the rightmost name.
    let calls = ts::extract_calls(Language::Kotlin, SRC);
    assert!(calls.iter().any(|c| c.name == "greet"));
}
