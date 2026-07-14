# pomelo-fmp

[![crates.io](https://img.shields.io/crates/v/pomelo-fmp)](https://crates.io/crates/pomelo-fmp)

Bring-your-own-key [FMP](https://site.financialmodelingprep.com/) (Financial
Modeling Prep) data sync + snapshot-factor formulas for the yuzu backtest
engine — part of [citrusquant](https://github.com/citrusquant/citrusquant).

Direct HTTP, **no third-party FMP SDK**. Given your own API key, fetch adjusted
daily bars (and optionally annual fundamentals, a `symbol → sector` industry map,
and snapshot-factor panels) and write a
[data-layout](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md)
tree:

```text
<out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (always)
<out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (--include-fundamentals)
<out>/tracked/universe.csv.gz    symbol,sector,market_cap       (--include-industry)
<out>/panels/{name}.csv.gz       snapshot-factor panels         (--include-snapshot-factors)
```

`sync` writes to a local path; `sync_into` is the storage-agnostic core over any
`ObjectSink` + `ObjectSource`, so the CLI and a backend service produce
byte-identical trees on local disk or an S3/R2 bucket (via `pomelo-s3`). The key
never leaves the machine; we neither host nor redistribute FMP data.

See the [FMP data source docs](https://github.com/citrusquant/citrusquant/blob/main/docs/fmp-data-source.md).

## License

MIT
