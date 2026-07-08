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
- Settings: `lemon.server.path`, `lemon.server.enabled`, `lemon.series`.
