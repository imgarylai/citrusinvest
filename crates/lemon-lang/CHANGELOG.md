# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/imgarylai/citrusinvest/compare/lemon-lang-v0.4.0...lemon-lang-v0.5.0) - 2026-07-10

### Added

- *(core)* [**breaking**] execution-layer stops (stop-loss/take-profit/trailing) + touched fills

## [0.4.0](https://github.com/imgarylai/citrusinvest/compare/lemon-lang-v0.3.0...lemon-lang-v0.4.0) - 2026-07-10

### Added

- *(lemon)* shareable strategy envelope format + `lemon check` validation
- *(core)* vol_target op — portfolio volatility targeting
- *(core)* cap_industry op — per-industry gross weight cap

## [0.3.0](https://github.com/imgarylai/citrusinvest/compare/lemon-lang-v0.2.0...lemon-lang-v0.3.0) - 2026-07-10

### Added

- *(ops)* Bollinger, MACD, Donchian + rolling_min surface
- *(ops)* in_sector membership mask from industry map
- *(ops)* cross-section winsorize, zscore, bucket, demean
- *(lemon)* expose exit_when and quantile_row on the DSL surface
- *(yuzu-py)* Python bindings via PyO3 (closes #25)

### Other

- Starter-tier data gaps vs citrusinvest features
- rustfmt for exit_when / quantile_row
- add input data layout for bring-your-own panels
- *(lemon)* rename crates/lemon directory to crates/lemon-lang

## [0.2.0](https://github.com/imgarylai/citrusinvest/compare/lemon-lang-v0.1.1...lemon-lang-v0.2.0) - 2026-07-08

### Added

- *(lemon)* editor language services, LSP server, and VS Code extension
- *(lemon)* semantic linter — unknown series and unused lets
- normalize_row op — explicit portfolio weights from a raw signal
- *(lemon)* `not` prefix operator
- *(lemon)* op catalog metadata + generated JSON schema/catalog

### Other

- not operator, normalize_row, YE freq, and lemon lint guide
- enforce 95% coverage gate and cover untested library paths
- add lemon language reference (docs/lemon.md) + fix DSL table + rustdoc
- crates.io keywords/categories, MSRV 1.86, CI + docs badges

## [0.1.1](https://github.com/imgarylai/citrusinvest/compare/lemon-lang-v0.1.0...lemon-lang-v0.1.1) - 2026-07-08

### Other

- add per-crate READMEs and crates.io badges
- release v0.1.0

## [0.1.0](https://github.com/imgarylai/citrusinvest/releases/tag/lemon-lang-v0.1.0) - 2026-07-08

### Other

- add CI, release-plz, and Pages docs; crates.io metadata
- rustfmt + clippy-clean the workspace
- make repo standalone — refresh docs, rebrand, wasm scripts to dist/OUT
- initial import of the citrusinvest engine
