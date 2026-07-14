# yuzu-research

[![crates.io](https://img.shields.io/crates/v/yuzu-research)](https://crates.io/crates/yuzu-research)

Multi-run backtest research over
[`yuzu-core`](https://crates.io/crates/yuzu-core) — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

`yuzu-core` runs one backtest; this crate is the layer above. It **composes** the
research primitives (factor IC/ICIR, event study, forward/daily returns) into
multi-run analyses over a synced data-layout tree — parameter **sweeps**,
**grids**, **walk-forward** selection, and **lookahead-bias** detection — plus
the data-loading glue that turns a `prices/` tree into an `EvalContext`.

It sits between the data/engine crates and the front ends
(`pomelo-*` / `yuzu-core` → `yuzu-research` → `yuzu-cli` or a backend service).
Everything returns serializable report structs; no CLI, no argument parsing.

See the [research docs](https://github.com/citrusquant/citrusquant/blob/main/docs/research.md).

## License

MIT
