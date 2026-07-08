# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

citrusinvest is a Rust backtest engine (a Cargo workspace):

- `yuzu-core` — pure, I/O-free backtest engine core (`run_backtest`).
- `yuzu-data` — native data loading (reads gzip CSV price/fundamental files into panels).
- `yuzu-source-s3` — an S3-backed data source (an `ObjectSource` impl; `LocalSource` reads local files).
- `lemon` / `lemon-lang` — the **strategy DSL**. Strategies are written in lemon and lowered to a JSON `Expr` tree the engine evaluates. Its `services` module provides pure editor language services (diagnostics/hover/completions).
- `lemon-lsp` — a thin `tower-lsp` language server over `lemon::services` (hover, completion, live diagnostics). Editor integration (incl. a VS Code extension + TextMate grammar) lives under `editors/`.

## Build · test · lint

```bash
cargo build --workspace          # build
cargo test --workspace           # test
cargo fmt --all                  # format (CI checks --check)
cargo clippy --workspace -- -D warnings   # lint (lib + bins)
bash scripts/build-yuzu-wasm.sh  # wasm -> dist/yuzu (build-lemon-wasm.sh for the DSL)
```

MSRV: **1.86**. Edition 2021.

## Writing or editing lemon strategies

If you generate or modify strategy code, **read [`docs/lemon.md`](docs/lemon.md)** — the full
language reference. The sharp edges that trip up generators:

- There is **no** `==`, `!=`, `&`, `|`, or `!`. Logical AND/OR/NOT are the words
  `and` / `or` / `not`; comparisons (`> < >= <=`) output `1.0`/`0.0`.
- Function calls take **positional args first, then keyword args** (`fn(x, n=3)`).
- List literals `[a, b]` exist **only** inside a call argument.
- An **unknown bare identifier silently becomes a data-series reference** — typos are not caught
  until engine evaluation. Valid series names (`close`, `high`, `volume`, `pe`, `roe`, …) are
  the engine's, not the parser's.
- A strategy is exactly one top-level expression (optionally preceded by `let name = …` bindings,
  which are inlined at parse time).

Machine-readable references for tool-use / structured output:

- [`schema/op-catalog.json`](schema/op-catalog.json) — every op with its arguments and defaults.
- [`schema/lemon-spec.schema.json`](schema/lemon-spec.schema.json) — JSON Schema for the spec tree.
- A compact prompt-ready cheat sheet: [`docs/lemon-prompt.md`](docs/lemon-prompt.md).

Validate generated lemon with the parser (`lemon::parse(src)` returns `ParseError { line, col, msg }`)
or the `lemon fmt` binary — feed it back to self-correct. `lemon lint --series close,pe,…`
additionally flags unknown series names (typos become silent `Data` leaves) and unused `let`s.

## Conventions

- **Conventional Commits** — release-plz derives versions/changelog from them. `feat:`→minor,
  `fix:`/`perf:`→patch; `docs`/`chore`/`refactor`/`test`/`ci` don't release.
- Don't hand-edit crate `version` fields — release-plz owns them.
- Published library crates: `yuzu-core`, `yuzu-data`, `yuzu-source-s3`, `lemon-lang`
  (the `lemon` crate is imported as `lemon`). The wasm/CLI/server crates are `publish = false`.
