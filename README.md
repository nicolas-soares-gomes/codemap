# codemap

A **deterministic** code index for AI agents. It builds a local, persisted graph (symbols,
ranges, relations) and serves an agent only the context it asks for — symbol-level navigation,
cheap in tokens, with **no manual grep** and **no confidence-based guessing**: every edge
carries a `provenance` (tree_sitter | stack_graphs | scip | lsp | text) and a `resolution`
(resolved | ambiguous | unresolved).

## Status

Work in progress.

- **M0 — Foundation** ✅: SQLite schema, core types, recursive-CTE traversal, migrations.
- **M1 — Tier0 tree-sitter (Rust, dogfood)** ✅: extraction, full-scan indexer, resolve/read-symbol/outline.
- **M2 — MCP server + compact projection** 🚧

## Architecture

Lean repo: a **single crate** (`codemap`, lib + bin), modules by responsibility.
On-disk SQLite storage (WAL, FTS5). Tiers: tree-sitter (always) · SCIP (opt-in) · LSP (on demand).

Firm policy: codemap **never installs** LSP/SCIP — it detects and instructs (`codemap doctor`).

## Usage (partial)

```
codemap index     # build ./.codemap/index.db (Tier0)
codemap resolve <name>     # name/name_path -> symbol ids
codemap outline <file>     # file symbols, no code
codemap read-symbol <id>   # one symbol's code (minimal range)
codemap doctor             # detect-only diagnostics
```

## License

MIT
