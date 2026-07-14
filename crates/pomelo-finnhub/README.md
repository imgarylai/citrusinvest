# pomelo-finnhub

[![crates.io](https://img.shields.io/crates/v/pomelo-finnhub)](https://crates.io/crates/pomelo-finnhub)

Bring-your-own-key [Finnhub](https://finnhub.io/) data sync for the yuzu backtest
engine — part of [citrusquant](https://github.com/citrusquant/citrusquant).

Direct HTTP, **no third-party Finnhub SDK**. Given your own API key, fetch market
data and write a
[data-layout](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md)
tree — the same contract as `pomelo-fmp` / `pomelo-eodhd` / `pomelo-alpha-vantage`:

```text
<out>/prices/{SYM}.csv.gz        adjusted OHLCV
<out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors
<out>/tracked/universe.csv.gz    symbol,sector,market_cap
<out>/panels/in_sp500.csv.gz     point-in-time SPX membership
<out>/panels/{factor}.csv.gz     best-effort snapshot factors
```

`sync` writes to a local path; `sync_into` is the storage-agnostic core over any
`ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`). The key
never leaves the machine; we neither host nor redistribute Finnhub data. Finnhub
has no clean delisted feed, so a Finnhub-only universe is survivor-biased —
documented, not faked.

See the [Finnhub data source docs](https://github.com/citrusquant/citrusquant/blob/main/docs/finnhub-data-source.md).

## License

MIT
