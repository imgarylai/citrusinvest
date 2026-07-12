---
title: API reference (docs.rs)
description: Where the generated Rust API documentation lives.
---

The Rust API reference is **generated per release and hosted on docs.rs** — the
canonical, versioned home for every published crate. We don't duplicate rustdoc
on this site; these links are always current with the latest release.

| Crate | API docs | What it is |
|-------|----------|-----------|
| `yuzu-core` | [docs.rs/yuzu-core](https://docs.rs/yuzu-core) | Pure, I/O-free backtest engine core. `run_backtest(spec_json, ctx, price_key, cfg) -> Report`. |
| `lemon-lang` | [docs.rs/lemon-lang](https://docs.rs/lemon-lang) | The lemon strategy DSL. `parse(src) -> Value`, `format(&value) -> String`. |
| `pomelo-data` | [docs.rs/pomelo-data](https://docs.rs/pomelo-data) | Native I/O: gzip CSV (and read-only Parquet, behind the `parquet` feature) price & fundamental files → panels. |
| `pomelo-s3` | [docs.rs/pomelo-s3](https://docs.rs/pomelo-s3) | S3-compatible `ObjectSource`/`ObjectSink` for `pomelo-data`. |
| `pomelo-fmp` | [docs.rs/pomelo-fmp](https://docs.rs/pomelo-fmp) | Bring-your-own-key FMP data sync + snapshot-factor formulas; writes to local disk or S3/R2. |
| `pomelo-audit` | [docs.rs/pomelo-audit](https://docs.rs/pomelo-audit) | Read-only data-quality audit of a data-layout tree (`yuzu-cli data-audit`). |
| `yuzu-research` | [docs.rs/yuzu-research](https://docs.rs/yuzu-research) | Multi-run research over the engine: sweeps, grids, walk-forward, lookahead-bias detection. |

The `wasm`, `cli`, `server`, and `lemon-lsp` crates are not published to
crates.io — they are application/tooling boundaries, not public library API.
Read their source in the
[repository](https://github.com/citrusquant/citrusquant/tree/main/crates). The
Python bindings (`yuzu-py`) aren't on crates.io either, but ship as the
[`yuzu-backtest`](https://pypi.org/project/yuzu-backtest/) wheel on PyPI.

## Machine-readable schemas

For tool-use and structured generation, two artifacts in the repo describe the
language precisely:

- [Lemon op catalog (JSON)](https://github.com/citrusquant/citrusquant/blob/main/schema/op-catalog.json)
  — every operator, its arguments, types, and defaults.
- [Lemon spec JSON Schema](https://github.com/citrusquant/citrusquant/blob/main/schema/lemon-spec.schema.json)
  — schema for the tagged `Expr` tree.
