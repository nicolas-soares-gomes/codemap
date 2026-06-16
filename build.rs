//! Compiles the vendored tree-sitter grammars whose published crates are incompatible with our
//! runtime (Kotlin, Clojure). Their generated C is linked directly via the stable parser ABI;
//! `src/ts/mod.rs` exposes the `tree_sitter_<lang>()` entry points as `LanguageFn`s.

use std::path::Path;

fn main() {
    compile_grammar("tree-sitter-kotlin", &["parser.c", "scanner.c"]);
    compile_grammar("tree-sitter-clojure", &["parser.c"]);
}

fn compile_grammar(dir: &str, sources: &[&str]) {
    let src = Path::new("vendor").join(dir).join("src");
    let mut build = cc::Build::new();
    build.include(&src).warnings(false).flag_if_supported("-w");
    for f in sources {
        let path = src.join(f);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }
    build.compile(&dir.replace('-', "_"));
}
