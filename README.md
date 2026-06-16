# codemap

A **deterministic** code index for AI agents. It builds a local, persisted graph (symbols,
ranges, relations) and serves an agent only the context it asks for — symbol-level navigation,
cheap in tokens, with **no manual grep** and **no confidence-based guessing**: every edge
carries a `provenance` (tree_sitter | stack_graphs | scip | lsp | text) and a `resolution`
(resolved | ambiguous | unresolved).

## How it works

Three layers, each more precise and more optional than the last:

- **tree-sitter** (always on): parses every file for symbols, ranges, outlines, and syntactic
  call edges. Works offline with zero setup.
- **SCIP** (optional): ingests a `.scip` index produced by an external indexer
  (`rust-analyzer scip`, `scip-typescript`, …) to upgrade call edges to precise/resolved.
- **language server** (optional, on demand): precise edges for languages without a good SCIP indexer.

codemap **never installs** those external tools — it detects them and prints the command to run
(`codemap doctor`, `codemap scip-cmd <lang>`).

Storage is a single on-disk SQLite file (`./.codemap/index.db`, WAL + FTS5). Navigation responses
never include source code; only `read-symbol` returns code — just the symbol's range, re-checked
against the file on disk first. Built as a single Rust crate (lib + bin).

## Languages

Rust, TypeScript, Python, Go, Java, C#, PHP, C, C++, Swift — each with its own extraction test.
(Kotlin and Clojure are pending: their tree-sitter grammars need to be vendored.)

## Commands

```
codemap index                          # build the index (tree-sitter)
codemap index --incremental            # only reindex what changed (git or mtime/size)
codemap index --with-scip --scip <f>   # also ingest a SCIP index for precise edges
codemap resolve <name>     # name -> symbol ids
codemap outline <file>     # symbols in a file
codemap read-symbol <id>   # one symbol's code (its range only)
codemap callers <sym>      # who calls a symbol
codemap callees <sym>      # what a symbol calls
codemap impact <sym>       # everything that breaks if you change it
codemap trace <sym>        # call chain up to entrypoints
codemap refs <sym>         # where a symbol is referenced
codemap variables <scope>  # fields/consts in a type or module
codemap search <query>     # search symbols by name
codemap export <sym> --format dot|mermaid
codemap watch              # reindex automatically as files change
codemap mcp                # serve the tools to an AI agent over stdio
codemap install [--hooks]  # teach your AI agent (Claude, Cursor, …) to use codemap
codemap status | prune | reset | doctor
```

## Agent tools (MCP)

`resolve_symbol`, `read_symbol`, `get_callers`, `get_callees`, `get_references`, `get_variables`,
`trace_to_roots`, `impact`, `search_code`, `get_file_outline`.

## License

MIT
