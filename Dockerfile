# Multi-stage build producing a static musl binary on a scratch image.
# tree-sitter (C) and SQLite (rusqlite "bundled") compile statically here.
#
# Caveat: SCIP indexers and LSP servers are EXTERNAL (Node, Go, .NET, rust-analyzer, ...) and
# are NOT in this image — codemap never installs them. The container indexes Tier0 on its own
# and can ingest a .scip you generated on the host/CI (mount it and run `index --tier1 --scip`).

FROM rust:1.95-alpine AS builder
RUN apk add --no-cache build-base musl-dev
WORKDIR /src
COPY . .
# On alpine the host target is musl (static by default). Our .cargo/config.toml forces
# +crt-static for cross-from-glibc builds, but here that flag would also hit proc-macros
# (which can't be static) — so drop it; the musl default still yields a static binary.
RUN rm -f .cargo/config.toml && cargo build --release --locked

FROM scratch AS runtime
COPY --from=builder /src/target/release/codemap /usr/local/bin/codemap
ENTRYPOINT ["/usr/local/bin/codemap"]
