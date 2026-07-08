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

## Install the language server

The extension launches the `lemon-lsp` binary. Build and install it from the
workspace root:

```bash
cargo install --path crates/lemon-lsp
```

This puts `lemon-lsp` on your `PATH`. If you keep it elsewhere, point the
extension at it with the `lemon.server.path` setting. To use highlighting only,
set `lemon.server.enabled` to `false`.

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
| `lemon.server.path`    | `lemon-lsp` | Path to the language server executable.                             |
| `lemon.server.enabled` | `true`      | Enable the language server (off = highlighting only).               |
| `lemon.series`         | `[]`        | Known data-series names; enables unknown-series diagnostics when set. |
