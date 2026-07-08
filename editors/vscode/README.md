# Lemon for VS Code

Editor support for the **lemon** strategy DSL (part of the
[citrusinvest](https://github.com/imgarylai/citrusinvest) engine):

- **Syntax highlighting** — a TextMate grammar (`syntaxes/lemon.tmLanguage.json`)
  for `.lemon` files. Works standalone, no server required.
- **Language server** — hover (op signatures + descriptions), completion (op
  names, keyword arguments, `let`-bound names, known series), and live
  diagnostics from the DSL linter (parse errors, unused `let`s, and — when
  `lemon.series` is configured — unknown-series warnings with did-you-mean
  suggestions), served by `lemon-lsp` over stdio.

Both surfaces are driven by the same single source of truth as the parser and
the JSON schema (`schema/op-catalog.json`), so the highlighting, hover text, and
completions never drift from the language itself.

## The language server

Hover, completion, and diagnostics are served by the `lemon-lsp` binary. The
extension finds it in this order:

1. an explicit `lemon.server.path`,
2. `lemon-lsp` on your `PATH`,
3. a previously downloaded copy, then
4. **auto-download** — the matching prebuilt binary is fetched from the project's
   GitHub Releases and cached (disable with `lemon.server.autoDownload`).

So on a supported platform (Linux/macOS/Windows, x64/arm64) it works with no
setup once binaries have been published. To use your own build instead:

```bash
cargo install --path crates/lemon-lsp   # puts lemon-lsp on PATH
```

Syntax highlighting always works with no server (`lemon.server.enabled: false`
for highlighting only).

> **Maintainers:** prebuilt binaries are produced by the `lemon-lsp binaries`
> GitHub Actions workflow (`.github/workflows/lemon-lsp-release.yml`). Run it
> (via *Run workflow* → tag, e.g. `lemon-lang-v0.2.0`) to cross-compile and
> attach `lemon-lsp-<platform>` assets to that release; the extension then
> downloads them automatically.

## Build the extension

```bash
cd editors/vscode
npm install
npm run compile          # esbuild bundle -> dist/extension.js
```

Then press <kbd>F5</kbd> in VS Code to launch an Extension Development Host (the
`Run Extension` launch config runs `npm run watch` for you), or package a
`.vsix` with [`vsce`](https://github.com/microsoft/vscode-vsce):

```bash
npm run package          # -> lemon-lang-<version>.vsix
```

Install the `.vsix` locally via **Extensions ▸ … ▸ Install from VSIX**, or
publish it (`npx @vscode/vsce publish`) once you have a registered publisher and
an Azure DevOps access token.

## Settings

| Setting                | Default     | Description                                                          |
| ---------------------- | ----------- | ------------------------------------------------------------------- |
| `lemon.server.path`         | `lemon-lsp` | Path to the language server executable.                              |
| `lemon.server.enabled`      | `true`      | Enable the language server (off = highlighting only).                |
| `lemon.server.autoDownload` | `true`      | Download a prebuilt `lemon-lsp` from GitHub Releases when not on PATH.|
| `lemon.series`              | `[]`        | Known data-series names; enables unknown-series diagnostics when set. |
