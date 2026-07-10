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
| `yuzu-data` | [docs.rs/yuzu-data](https://docs.rs/yuzu-data) | Native I/O: gzip CSV / Parquet price & fundamental files → panels. |
| `yuzu-source-s3` | [docs.rs/yuzu-source-s3](https://docs.rs/yuzu-source-s3) | S3-compatible `ObjectSource` for `yuzu-data`. |

The `wasm`, `cli`, and `server` crates are not published to crates.io — they are
application boundaries, not public library API. Read their source in the
[repository](https://github.com/imgarylai/citrusinvest/tree/main/crates).

## Machine-readable schemas

For tool-use and structured generation, two artifacts in the repo describe the
language precisely:

- [Lemon op catalog (JSON)](https://github.com/imgarylai/citrusinvest/blob/main/schema/op-catalog.json)
  — every operator, its arguments, types, and defaults.
- [Lemon spec JSON Schema](https://github.com/imgarylai/citrusinvest/blob/main/schema/lemon-spec.schema.json)
  — schema for the tagged `Expr` tree.
