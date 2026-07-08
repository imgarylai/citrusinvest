# yuzu-data

[![crates.io](https://img.shields.io/crates/v/yuzu-data)](https://crates.io/crates/yuzu-data)

Native I/O layer for [citrusinvest](https://github.com/imgarylai/citrusinvest):
reads price and fundamental files into
[`yuzu-core`](https://crates.io/crates/yuzu-core) panels.

## Formats

The file format is detected from content, not the object key, so existing data
keeps working and mirrors can migrate incrementally:

- **gzip CSV** (`.csv.gz`) — the default write format.
- **plain CSV** (`.csv`).
- **Apache Parquet** (`.parquet`) — columns matched by name (`day`, `adj_close`,
  `pe`, …; one column per symbol for combined panels); the `day` column may be an
  integer `YYYYMMDD`, a `YYYY-MM-DD` string, or a logical `date`. Reading only —
  writing/rebuild output stays gzip CSV.

All three work out of the box; the loaders probe `.csv.gz`, then `.parquet`,
then `.csv` for each symbol/panel.

Parquet pulls the arrow stack. Consumers that only need CSV/gzip and want to
avoid that dependency can opt out:

```toml
yuzu-data = { version = "0.1", default-features = false }
```

See the [engine docs](https://github.com/imgarylai/citrusinvest/blob/main/docs/backtest-engine.md).

## License

MIT
