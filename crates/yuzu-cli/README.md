# yuzu-cli

Native batch backtest runner CLI over a locally-synced price mirror — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

A thin layer over the engine, data, and research crates: `main.rs` is the
[clap](https://crates.io/crates/clap) front end, and the actual logic lives in
the libraries — the multi-run research orchestration (sweep / grid / walk-forward
/ lookahead) in [`yuzu-research`](https://crates.io/crates/yuzu-research),
re-exported here so callers keep the `yuzu_cli::…` path. Not published to
crates.io.

See the [research docs](https://github.com/citrusquant/citrusquant/blob/main/docs/research.md)
and the [engine docs](https://github.com/citrusquant/citrusquant/blob/main/docs/backtest-engine.md).

## License

MIT
