# lemon-lsp

Language Server Protocol server for the **lemon** strategy DSL — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

A thin [`tower-lsp`](https://crates.io/crates/tower-lsp) shim over
`lemon::services`: every bit of language intelligence — diagnostics, hover,
completions — is a pure, unit-tested function in the `lemon` library. This binary
only translates between the LSP wire types and those functions and runs the stdio
event loop. Editors launch it over stdio:

```text
lemon-lsp
```

Not published to crates.io.

See the [lemon DSL docs](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md).

## License

MIT
