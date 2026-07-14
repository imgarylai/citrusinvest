# pomelo-audit

[![crates.io](https://img.shields.io/crates/v/pomelo-audit)](https://crates.io/crates/pomelo-audit)

Read-only data-quality audit of a pomelo data-layout tree — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

Given a synced `prices/` / `fundamentals/` / `panels/` / `tracked/` tree,
`run_data_audit` answers *"is this clean enough to trust a backtest?"* — turning
"high-quality data" from a claim into a measurement: coverage, calendar gaps,
adjustment sanity, survivorship, NaN density, filing-date lag, and index
membership, each reported as an `OK` / `WARN` / `FAIL` verdict.

Storage-agnostic: it takes any `ObjectSource` + `ObjectLister`, so it audits a
local tree or an S3/R2 tree identically. No network beyond the source's own
reads, no engine run — it reuses the `pomelo-data` loaders and returns a
serializable `DataAuditReport` any front end can render (the `yuzu-cli
data-audit` CLI is a thin shim that maps a `FAIL` to a non-zero exit).

See the [data sources overview](https://github.com/citrusquant/citrusquant/blob/main/docs/data-sources.md).

## License

MIT
