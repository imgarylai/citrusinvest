# pomelo-eodhd

[![crates.io](https://img.shields.io/crates/v/pomelo-eodhd)](https://crates.io/crates/pomelo-eodhd)

Bring-your-own-key [EODHD](https://eodhd.com/) data sync for the yuzu backtest
engine — part of [citrusquant](https://github.com/citrusquant/citrusquant).

Direct HTTP, **no third-party EODHD SDK**. Given your own API token, fetch market
data and write a
[data-layout](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md)
tree — the same contract as `pomelo-fmp`:

```text
<out>/prices/{SYM}.csv.gz        adjusted OHLCV
<out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors
<out>/tracked/universe.csv.gz    symbol,sector,market_cap
<out>/panels/{name}.csv.gz       membership / snapshot panels
```

`sync` writes to a local path; `sync_into` is the storage-agnostic core over any
`ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`). The token
never leaves the machine; we neither host nor redistribute EODHD data.

See the [EODHD data source docs](https://github.com/citrusquant/citrusquant/blob/main/docs/eodhd-data-source.md).

## License

MIT
