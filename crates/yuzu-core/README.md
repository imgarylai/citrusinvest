# yuzu-core

[![crates.io](https://img.shields.io/crates/v/yuzu-core)](https://crates.io/crates/yuzu-core)

Pure, I/O-free backtest engine core for US equity strategies — part of
[citrusinvest](https://github.com/imgarylai/citrusinvest).

Evaluates a strategy spec plus data panels into a `Report`:
`yuzu_core::run_backtest(spec_json, ctx, price_key, cfg)`. No network, no platform
dependencies; compiles to both native and WASM.

See the [engine docs](https://github.com/imgarylai/citrusinvest/blob/main/docs/backtest-engine.md).

## License

MIT
