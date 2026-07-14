# yuzu-wasm

WASM JSON boundary over
[`yuzu-core`](https://crates.io/crates/yuzu-core)'s backtest, for in-browser
execution — part of [citrusquant](https://github.com/citrusquant/citrusquant).

`run_backtest_json` is pure Rust (unit-tested here); the `wasm_bindgen` export
re-exposes it to the browser Worker. String in, string out:

```text
input:  { spec, panels:{name:{dates,symbols,data}}, industry?, price_key, config? }
output: Report JSON
```

`data` cells may be `null` (→ NaN). Not published to crates.io — it is built into
the web app's WASM bundle.

See the [engine docs](https://github.com/citrusquant/citrusquant/blob/main/docs/backtest-engine.md).

## License

MIT
