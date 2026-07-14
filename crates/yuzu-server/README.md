# yuzu-server

HTTP backtest server that reads only the series a spec references and runs
[`yuzu-core`](https://crates.io/crates/yuzu-core) — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

`handle_backtest` is the source-agnostic unit: it reads only the price/fundamental
series a spec references (from any `ObjectSource` — `LocalSource` in tests,
`S3Source`/R2 in the container), assembles an `EvalContext`, and runs
`yuzu_core::run_backtest`. `main.rs` wraps it in a tiny HTTP server; no Cloudflare
specifics live here. Not published to crates.io.

See the [engine docs](https://github.com/citrusquant/citrusquant/blob/main/docs/backtest-engine.md).

## License

MIT
