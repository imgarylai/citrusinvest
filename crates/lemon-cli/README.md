# lemon-cli

The `lemon` command-line tool for the lemon strategy DSL — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

- `lemon fmt` — canonical formatter (parses first, so it doubles as a syntax
  checker)
- `lemon lint` — semantic warnings: unknown series names, unused `let` bindings
- `lemon check` — validate shareable [strategy envelopes](https://github.com/citrusquant/citrusquant/blob/main/docs/strategy-envelope.md)
  and `.lemon` files (front-matter + syntax)
- `lemon run` — lower a `.lemon` file (or an envelope) and run a native
  backtest over a local
  [data-layout tree](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md),
  printing the engine's Report JSON. `lemon strategy.lemon` is short for it:

  ```bash
  lemon run strategy.lemon --data ~/qdata --from 20180101 --fee-ratio 0.001
  lemon strategy.lemon    # parameters from `#!` front-matter + $CITRUS_DATA
  ```

  Run parameters can live in the file itself as `#!` front-matter (`name`,
  `universe`, `config`, `price-key`) — precedence is flag > document >
  environment > default. See the
  [lemon reference](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md)
  for the full syntax.

The language itself lives in [`lemon-lang`](../lemon-lang) (imported as
`lemon`), which stays pure and I/O-free; this crate is the thin native shim
that wires the parser to `yuzu-research`'s backtest runner. Not published to
crates.io — install from a checkout:

```bash
cargo install --path crates/lemon-cli
```

See the [lemon language reference](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md).

## License

MIT
