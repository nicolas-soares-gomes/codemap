Use the `codemap` CLI to LOCATE and TRACE code by symbol — it replaces grep for "where is X / who
calls X / what does a change touch". It returns ids + `file:start-end` line ranges + relations, NOT
bulk code. Once it points you at `file:start-end`, read EXACTLY those lines — never the whole file
just to inspect a symbol.

1. Locate / trace (use codemap instead of grep; returns locations, no code):
   - codemap search <q> [--mode text]   find symbols by name (prefix; --mode text = substring)
   - codemap resolve <name>             exact name or Type/method -> stable ids + file:start-end
   - codemap grep <regex>               find a value/string/regex in file CONTENTS, mapped to its symbol
   - codemap outline <file>             a file's symbols (skim its shape before reading)
   - codemap callers <sym> / callees <sym> / refs <sym>   who calls it / what it calls / where it's used
   - codemap impact <sym>               transitive callers — what breaks if you change it
   - codemap trace <sym>                call chain up to entrypoints

2. Understand: codemap rows give `file:start-end`. Read EXACTLY that range — `read-symbol <id>`
   returns precisely those lines, or use your Read tool with offset=start and limit≈(end-start+1)
   (add a few context lines if needed). Do NOT read the whole file just to inspect one symbol; that
   is the main way to waste tokens. Read more only when a range genuinely spans it, or when you must
   verify a path the relations surfaced.

3. Edit, then keep the index fresh: `codemap index --incremental` (or rely on the git hooks).

Rules:
- Find WHERE something is, WHO uses it, or WHAT a change impacts -> codemap, not grep.
- Understand HOW it works -> read the exact `file:start-end` range, not the whole file.
- Each edge carries prov/res (scip|lsp|tree_sitter / resolved|ambiguous|unresolved). Prefer resolved
  edges; treat ambiguous ones as a lead to verify, not as truth.
- For a value/string/regex inside the code (URL, env key, magic constant), use `codemap grep`.
