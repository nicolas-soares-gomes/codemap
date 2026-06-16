# codemap

A **deterministic** code index for AI agents. It builds a local, persisted graph (symbols, ranges,
relations) and serves your agent only the context it asks for — symbol-level navigation, cheap in
tokens, with **no manual grep** and **no confidence-based guessing**: every edge carries a
`provenance` (tree_sitter | scip | lsp | text) and a `resolution` (resolved | ambiguous | unresolved).

## Quickstart

Two steps: install the binary, then run `setup` in your repo.

```sh
# 1. install (pick one)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nicolas-soares-gomes/codemap/releases/latest/download/codemap-installer.sh | sh   # macOS / Linux
brew install nicolas-soares-gomes/tap/codemap                                                                                            # Homebrew
cargo install --git https://github.com/nicolas-soares-gomes/codemap                                                                      # from source

# Windows (PowerShell):
#   irm https://github.com/nicolas-soares-gomes/codemap/releases/latest/download/codemap-installer.ps1 | iex

# 2. in your project
cd my-project
codemap setup --index --hooks
```

`codemap setup`:
- checks the repo and tells you which optional language tools unlock precise results (it never
  installs them for you),
- **teaches your coding agent** — by writing a skill into the files it reads (`AGENTS.md`,
  `GEMINI.md`, `.claude/`, `.cursor/`, `.github/`) — to navigate by symbol instead of grepping.

`--index` builds the index now; `--hooks` keeps it fresh on every commit. After that you're done —
your **agent** just runs the `codemap` commands below in its shell; you don't run them by hand.
No MCP, no per-agent config: any agent that can run a shell command can use codemap.

## How it works

Three layers, each more precise and more optional than the last:

- **tree-sitter** (always on): parses every file for symbols, ranges, outlines, and syntactic call
  edges. Offline, zero setup.
- **SCIP** (optional): ingests a `.scip` index from an external indexer (`rust-analyzer scip`,
  `scip-typescript`, …) to upgrade call edges to precise/resolved.
- **language server** (optional, on demand): precise edges for languages without a good SCIP indexer.

codemap **never installs** those external tools — it detects them and prints the command to run.
Storage is a single SQLite file (`./.codemap/index.db`). Navigation responses never include source
code; only `read-symbol` returns code — just the symbol's range, re-checked against disk first.

In a polyglot monorepo, each build root (`package.json`, `Cargo.toml`, `go.mod`, …) is an **index
unit**; `codemap status` reports per-unit resolution coverage.

## Languages

Rust, TypeScript, Python, Go, Java, C#, PHP, C, C++, Swift, Kotlin, Clojure.

## Agent commands

codemap is a CLI — the agent runs these in its shell. The skill installed by `setup` teaches the
cheap→expensive ladder; every command returns compact, code-free rows except `read-symbol`.

```
codemap resolve <name>      # exact name / Type.method -> stable ids
codemap search <query>      # name prefix search -> ids (--mode text for a substring match)
codemap grep <regex>        # search file CONTENTS for a value/string/regex, mapped to its symbol
codemap outline <file>      # a file's symbols, instead of reading it
codemap callers <sym>       # who calls it
codemap callees <sym>       # what it calls
codemap refs <sym>          # every use, resolved to the enclosing symbol
codemap impact <sym>        # transitive callers — what breaks if you change it
codemap trace <sym>         # call chain up to entrypoints
codemap variables <scope>   # fields/consts of a type or module
codemap read-symbol <id>    # the only command that returns code (its range only)
codemap export <sym> --format dot|mermaid
```

Index management (you, or the git hooks from `setup --hooks`):

```
codemap index [--incremental] [--scip <f> ...]    # build / refresh the index
codemap status | doctor | prune | reset | watch
```

### Build features (optional)

- `--features tier2-lsp` → `codemap lsp-enrich <symbol>` (confirm a symbol's edges via a
  user-installed language server; codemap never installs it).

## Contributing

```sh
brew install lefthook   # or: npm i -g lefthook / go install / mise install
lefthook install        # wires the git hooks
```

`lefthook` runs `cargo fmt --check` + `cargo clippy -D warnings` on commit and `cargo test` on push
(see `lefthook.yml`). Bypass a hook with `git commit --no-verify`. CI runs the same checks.

## License

MIT
