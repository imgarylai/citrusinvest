# Change Log

All notable changes to the Lemon extension are documented here.

## [0.1.0]

Initial release.

- Syntax highlighting for `.lemon` files (TextMate grammar).
- Language server client for `lemon-lsp`: hover (op signatures + descriptions),
  completion (op names, keyword arguments, `let`-bound names, known series), and
  live diagnostics from the DSL linter (parse errors, unused `let`s, and — when
  `lemon.series` is configured — unknown-series warnings with did-you-mean
  suggestions).
- Automatic server install: when `lemon-lsp` is not on `PATH`, the matching
  prebuilt binary is downloaded from GitHub Releases and cached
  (`lemon.server.autoDownload`).
- Settings: `lemon.server.path`, `lemon.server.enabled`,
  `lemon.server.autoDownload`, `lemon.series`.
