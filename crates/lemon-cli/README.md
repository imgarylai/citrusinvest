# lemon-cli

The `lemon` command-line tool for the lemon strategy DSL — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

- `lemon new` — scaffold a starter `.lemon` (commented front-matter + a runnable
  example) to stdout or a path
- `lemon fmt` — canonical formatter (parses first, so it doubles as a syntax
  checker)
- `lemon lint` — semantic warnings: unknown series names, unused `let` bindings
- `lemon check` — validate shareable [strategy envelopes](https://github.com/citrusquant/citrusquant/blob/main/docs/strategy-envelope.md)
  and `.lemon` files (front-matter + syntax)
- `lemon run` — lower a `.lemon` file (or an envelope) and run a native
  backtest over a local
  [data-layout tree](https://github.com/citrusquant/citrusquant/blob/main/docs/data-layout.md),
  printing a human-readable metrics summary (or the full Report JSON with
  `--json`). `lemon strategy.lemon` is short for it:

  ```bash
  lemon run strategy.lemon --data ~/qdata --from 20180101 --fee-ratio 0.001
  lemon strategy.lemon    # parameters from `#!` front-matter + $CITRUS_DATA
  ```

  Run parameters can live in the file itself as `#!` front-matter (`name`,
  `universe`, `symbols`, `require`, `data-source`, `config`, `price-key`) —
  precedence is flag > document > environment > default. With `--sync` (and
  your own `$FMP_API_KEY`), missing symbols are fetched through the file's
  declared `data-source` before the run. See the
  [lemon reference](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md)
  for the full syntax.

The language itself lives in [`lemon-lang`](../lemon-lang) (imported as
`lemon`), which stays pure and I/O-free; this crate is the thin native shim
that wires the parser to `yuzu-research`'s backtest runner.

## Install

Prebuilt binaries (Linux / macOS; checksum-verified):

```bash
curl -fsSL https://citrusquant.com/install.sh | sh
```

Windows: grab `lemon-x86_64-pc-windows-msvc.exe` from the
[releases page](https://github.com/citrusquant/citrusquant/releases). The crate
is not published to crates.io — from source, install from a checkout:

```bash
cargo install --path crates/lemon-cli
```

See the [lemon language reference](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md).

## License

MIT
