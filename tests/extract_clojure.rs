//! Symbol extraction test for CLOJURE (vendored grammar). The grammar is generic, so definitions
//! are recognized by their head symbol and namespaced under the file's `ns`.

use codemap::ts::{self, Extracted};
use codemap::types::{Language, SymbolKind};

const SRC: &str = "\
(ns my.app)
(def pi 3.14)
(defn helper [x] (* x 2))
(defn run [y] (helper y))
(defrecord Point [x y])
(defmacro unless [c body] body)
";

fn has(syms: &[Extracted], name_path: &str, kind: SymbolKind) -> bool {
    syms.iter()
        .any(|s| s.name_path == name_path && s.kind == kind)
}

#[test]
fn extracts_clojure_symbols_namespaced() {
    let syms = ts::extract(Language::Clojure, SRC);
    assert!(has(&syms, "my.app", SymbolKind::Module));
    assert!(has(&syms, "my.app/pi", SymbolKind::Const));
    assert!(has(&syms, "my.app/helper", SymbolKind::Function));
    assert!(has(&syms, "my.app/run", SymbolKind::Function));
    assert!(has(&syms, "my.app/Point", SymbolKind::Struct));
    assert!(has(&syms, "my.app/unless", SymbolKind::Macro));
}

#[test]
fn extracts_clojure_calls() {
    let src = "(defn callee [] 1)\n(defn caller [] (callee))\n";
    let calls = ts::extract_calls(Language::Clojure, src);
    assert!(calls.iter().any(|c| c.name == "callee"));
    // The def forms themselves are definitions, not calls.
    assert!(!calls.iter().any(|c| c.name == "defn"));
}
