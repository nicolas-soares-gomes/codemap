-- codemap schema v1 (SQLite >= 3.45). Pragmas (WAL, foreign_keys, cache, mmap) are
-- applied in code, not here.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT
) WITHOUT ROWID;
-- keys: schema_version, indexed_commit, scanner_mode ('git'|'fs'), codemap_version, repo_root

CREATE TABLE string_pool (
  id       INTEGER PRIMARY KEY,
  text     TEXT NOT NULL,
  refcount INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX ux_string_pool_text ON string_pool(text);

CREATE TABLE file (
  id             INTEGER PRIMARY KEY,
  path_sid       INTEGER NOT NULL REFERENCES string_pool(id),
  lang           INTEGER NOT NULL,
  size           INTEGER NOT NULL,
  mtime_ns       INTEGER NOT NULL,
  content_hash   BLOB    NOT NULL,
  line_count     INTEGER NOT NULL,
  indexed_at     INTEGER NOT NULL,
  tier           INTEGER NOT NULL DEFAULT 0,   -- 0=tree-sitter 1=scip 2=lsp-enriched
  index_unit_sid INTEGER REFERENCES string_pool(id)
);
CREATE UNIQUE INDEX ux_file_path ON file(path_sid);

-- Per-line byte offsets, so a symbol range can be read from disk without storing code.
CREATE TABLE line_index (
  file_id INTEGER PRIMARY KEY REFERENCES file(id) ON DELETE CASCADE,
  offsets BLOB NOT NULL
) WITHOUT ROWID;

CREATE TABLE symbol (
  id            INTEGER PRIMARY KEY,
  symbol_key    BLOB NOT NULL,                 -- blake3(path + name_path + kind + overload-disambiguator)
  file_id       INTEGER NOT NULL REFERENCES file(id) ON DELETE CASCADE,
  name_sid      INTEGER NOT NULL REFERENCES string_pool(id),
  name_path_sid INTEGER NOT NULL REFERENCES string_pool(id),
  scip_sym_sid  INTEGER REFERENCES string_pool(id),
  signature_sid INTEGER REFERENCES string_pool(id),
  kind          INTEGER NOT NULL,
  parent_id     INTEGER REFERENCES symbol(id) ON DELETE CASCADE,
  start_line    INTEGER NOT NULL,
  start_col     INTEGER NOT NULL,
  end_line      INTEGER NOT NULL,
  end_col       INTEGER NOT NULL,
  sel_line      INTEGER NOT NULL,              -- selectionRange (the name itself)
  sel_col       INTEGER NOT NULL
);
CREATE UNIQUE INDEX ux_symbol_key      ON symbol(symbol_key);
CREATE INDEX        ix_symbol_file     ON symbol(file_id);
CREATE INDEX        ix_symbol_name     ON symbol(name_sid);
CREATE INDEX        ix_symbol_namepath ON symbol(name_path_sid);
CREATE INDEX        ix_symbol_parent   ON symbol(parent_id);
CREATE INDEX        ix_symbol_scip     ON symbol(scip_sym_sid) WHERE scip_sym_sid IS NOT NULL;

CREATE TABLE occurrence (
  id           INTEGER PRIMARY KEY,
  symbol_id    INTEGER REFERENCES symbol(id) ON DELETE SET NULL,   -- resolved target (NULL=unresolved)
  enclosing_id INTEGER REFERENCES symbol(id) ON DELETE SET NULL,   -- containing symbol (derives caller)
  file_id      INTEGER NOT NULL REFERENCES file(id) ON DELETE CASCADE,
  role         INTEGER NOT NULL,              -- 0=def 1=ref 2=read 3=write 4=call
  start_line   INTEGER NOT NULL,
  start_col    INTEGER NOT NULL,
  end_line     INTEGER NOT NULL,
  end_col      INTEGER NOT NULL
);
CREATE INDEX ix_occ_symbol ON occurrence(symbol_id, role, enclosing_id);

-- Deduplicated relation. occurrence_id is intentionally NOT in the PK: WITHOUT ROWID forces
-- PK columns NOT NULL, and most tree-sitter edges have no call-site. Call-sites go in call_site.
CREATE TABLE edge (
  source_symbol_id INTEGER NOT NULL REFERENCES symbol(id) ON DELETE CASCADE,
  target_symbol_id INTEGER NOT NULL REFERENCES symbol(id) ON DELETE CASCADE,
  kind             INTEGER NOT NULL,          -- see EdgeKind
  provenance       INTEGER NOT NULL,          -- 0=tree_sitter 1=stack_graphs 2=scip 3=lsp 4=text
  resolution       INTEGER NOT NULL,          -- 0=resolved 1=ambiguous 2=unresolved
  PRIMARY KEY (source_symbol_id, target_symbol_id, kind)
) WITHOUT ROWID;
CREATE INDEX ix_edge_target ON edge(target_symbol_id, kind);

CREATE TABLE call_site (
  source_symbol_id INTEGER NOT NULL,
  target_symbol_id INTEGER NOT NULL,
  kind             INTEGER NOT NULL,
  occurrence_id    INTEGER NOT NULL REFERENCES occurrence(id) ON DELETE CASCADE,
  PRIMARY KEY (source_symbol_id, target_symbol_id, kind, occurrence_id),
  FOREIGN KEY (source_symbol_id, target_symbol_id, kind)
    REFERENCES edge(source_symbol_id, target_symbol_id, kind) ON DELETE CASCADE
) WITHOUT ROWID;

-- Contentless: rowid = symbol.id. CASCADE does NOT clean this index; the writer must emit a
-- 'delete' with the OLD column values before deleting a symbol.
CREATE VIRTUAL TABLE symbol_fts USING fts5(name, name_path, content='');
