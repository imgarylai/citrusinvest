# lemon-wasm

WASM boundary for the **lemon** strategy DSL — part of
[citrusquant](https://github.com/citrusquant/citrusquant).

Parses `.lemon` source ⇄ JSON `Expr` tree and exposes the editor language
services (diagnostics, hover, completions) that back the in-browser editor.
String in, string out — mirrors `yuzu-wasm`'s JSON boundary. The pure functions
are unit-tested natively; the `#[wasm_bindgen]` wrappers are wasm32-gated. Not
published to crates.io — it is built into the web app's WASM bundle.

See the [lemon DSL docs](https://github.com/citrusquant/citrusquant/blob/main/docs/lemon.md).

## License

MIT
