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
- **M2 — Graph + MCP + skills** ✅: stable SymbolId, Tier0 call edges, callers/callees, MCP server (rmcp/stdio), multi-platform skill installer.
- **M3 — SCIP Tier1** ✅: `doctor` capability matrix + SCIP ingestion (resolved call edges; verified with real `rust-analyzer scip`).
- **M4 — Incremental / watcher** ✅: git-hook-driven & manual incremental reconcile, toggleable watcher, inline staleness guard.
- **M5 — Export / Docker / languages** 🚧: DOT/Mermaid export ✅, static musl Docker image (scratch, ~15 MB) ✅, git-hooks installer ✅; more languages ongoing.

Tier0 languages: **Rust, TypeScript, Python, Go, Java, C#, PHP, C, C++** (each with a per-language
extraction test). Kotlin, Swift, and Clojure are pending (pre-1.0 / variable grammars).

## Architecture

Lean repo: a **single crate** (`codemap`, lib + bin), modules by responsibility.
On-disk SQLite storage (WAL, FTS5). Tiers: tree-sitter (always) · SCIP (opt-in) · LSP (on demand).

Firm policy: codemap **never installs** LSP/SCIP — it detects and instructs (`codemap doctor`).

## Usage (partial)

```
codemap index              # build ./.codemap/index.db (Tier0)
codemap resolve <name>     # name/name_path -> symbol ids
codemap outline <file>     # file symbols, no code
codemap read-symbol <id>   # one symbol's code (minimal range)
codemap callers <sym>      # who calls a symbol (resolved edges)
codemap callees <sym>      # what a symbol calls
codemap mcp                # MCP server over stdio (for agents)
codemap install            # write the codemap skill into detected agent hosts
codemap doctor             # detect-only diagnostics
```

## License

MIT
