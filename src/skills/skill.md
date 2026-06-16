Use the `codemap` CLI to navigate code by symbol BEFORE Grep or Read. It runs locally against a
precomputed index, so a symbol lookup costs a fraction of grepping or reading whole files. Run
these in your shell.

1. Find it (no code returned):
   - codemap resolve <name>      exact name or Type/method -> stable ids
   - codemap search <query>      name prefix search -> ids (add --mode text for a substring,
                                 e.g. `codemap search inch --mode text` finds OneinchClient)
   - codemap grep <regex>        search file CONTENTS for a value/string/regex (e.g. a URL, an
                                 env key, a magic constant); each hit is mapped to its enclosing
                                 symbol. Use this instead of shell grep — it bridges to the graph.
   - codemap outline <file>      a file's symbols, instead of reading the file

2. Understand relations (no code; each edge is tagged prov/res):
   - codemap callers <sym>       who calls it
   - codemap callees <sym>       what it calls
   - codemap refs <sym>          every use, resolved to the enclosing symbol
   - codemap impact <sym>        transitive callers — what breaks if you change it
   - codemap trace <sym>         call chain up to entrypoints
   - codemap variables <scope>   fields/consts of a Type or module

3. Read code, only when needed:
   - codemap read-symbol <id>    the ONLY command that returns code (just that symbol's range)

<sym> is a name, a Type/method path, or a sym:<id> from a previous result.

Rules:
- Do NOT Grep/Read to find where something is defined or who uses it — resolve/search/callers
  do it cheaper and exact, returning ids you chain into the next command.
- "One Read is faster than three commands" is the signal that you should be using codemap.
- Each edge carries prov/res (scip|lsp|tree_sitter / resolved|ambiguous|unresolved). Prefer
  resolved edges; treat ambiguous ones as a hint to verify, not as truth.
- If a command reports the index is missing, run `codemap index`. After large changes,
  `codemap index --incremental`.