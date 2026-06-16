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
- **M4 — Incremental / watcher** ✅: git fast-path reconcile (diff `indexed_commit..HEAD`) + mtime/size fallback, toggleable watcher, inline staleness guard, git-hooks installer.
- **M5 — Export / Docker / languages** ✅: DOT/Mermaid export, static musl Docker image (scratch, ~15 MB), 10 Tier0 languages.

All 10 MCP tools are implemented: `resolve_symbol`, `read_symbol`, `get_callers`, `get_callees`,
`get_references`, `get_variables`, `trace_to_roots`, `impact`, `search_code`, `get_file_outline`.

Tier0 languages: **Rust, TypeScript, Python, Go, Java, C#, PHP, C, C++, Swift** (each with a
per-language extraction test). Kotlin and Clojure are blocked by their tree-sitter grammars
(kotlin-ng mis-parses basic classes; tree-sitter-clojure pins an incompatible tree-sitter crate).

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
codemap impact <sym>       # transitive callers — what breaks if you change it
codemap trace <sym>        # call chain up to root entrypoints
codemap refs <sym>         # references resolved to the enclosing symbol
codemap variables <scope>  # fields/consts under a type/module
codemap search <query>     # FTS5 symbol search
codemap export <sym> --format dot|mermaid
codemap index --incremental [--tier1 --scip <f>]
codemap watch              # live incremental reindex
codemap mcp                # MCP server over stdio (for agents)
codemap install [--hooks]  # write the codemap skill into detected agent hosts
codemap doctor             # detect-only diagnostics
```

## License

MIT
