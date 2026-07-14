# pomelo-alpha-vantage

[![crates.io](https://img.shields.io/crates/v/pomelo-alpha-vantage)](https://crates.io/crates/pomelo-alpha-vantage)

Bring-your-own-key [Alpha Vantage](https://www.alphavantage.co/) data sync for
the yuzu backtest engine — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

Direct HTTP, **no third-party Alpha Vantage SDK**. Given your own API key, fetch
market data and write a
[data-layout](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md)
tree — the same contract as `pomelo-fmp` / `pomelo-eodhd`:

```text
<out>/prices/{SYM}.csv.gz        adjusted OHLCV
<out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors
<out>/tracked/universe.csv.gz    symbol,sector,market_cap
```

`sync` writes to a local path; `sync_into` is the storage-agnostic core over any
`ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`). The key
never leaves the machine; we neither host nor redistribute Alpha Vantage data.
Alpha Vantage has no historical index constituents, so there is **no faked**
point-in-time membership panel.

See the [Alpha Vantage data source docs](https://github.com/citrusquant/citrusquant/blob/main/docs/alpha-vantage-data-source.md).

## License

MIT
