# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.8.2...yuzu-core-v0.9.0) - 2026-07-13

### Fixed

- *(yuzu-core)* NaN-safe sorts and non-panicking date decode
- correct doc drift (rise/fall, op counts, crate publish list, Parquet)

### Other

- *(yuzu-core)* proptest edge cases and document NaN policy
- *(yuzu-core)* fix cap_industry doc link to private helper
- *(yuzu-core)* expand rustdoc for public APIs
- rustfmt EngineError call sites
- *(yuzu-core)* structured EngineError via thiserror
- rustfmt bootstrap comment alignment

## [0.8.1](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.7.0...yuzu-core-v0.8.1) - 2026-07-12

### Other

- release v0.7.1

## [0.7.0](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.6.0...yuzu-core-v0.7.0) - 2026-07-12

### Other

- *(yuzu-research)* extract multi-run research orchestration from the CLI
- *(audit)* move data-audit into a pomelo-audit crate; CLI just calls it

## [0.6.0](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.5.1...yuzu-core-v0.6.0) - 2026-07-12

### Other

- address #139 review — three-family docs + restore CHANGELOG tags
- *(data)* [**breaking**] rename yuzu-data → pomelo-data, yuzu-source-s3 → pomelo-s3
- *(data)* extract pomelo-fmp crate (FMP sync + factor formulas)
- *(yuzu-core)* borrow Data leaves in eval via EvalOut

## [0.5.1](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.5.0...yuzu-core-v0.5.1) - 2026-07-10

### Fixed

- *(clippy)* allow too_many_arguments on run_with_initial wrapper

### Other

- *(yuzu-core)* shared stat helpers and eval by op family
- split backtest.rs and fmp.rs into module trees
- citrusinvest -> citrusquant (brand, URLs, org/repo, site domain)

## [0.5.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-core-v0.4.0...yuzu-core-v0.5.0) - 2026-07-10

### Added

- *(server,wasm)* expose execution-layer stops in the request configs
- *(core)* [**breaking**] execution-layer stops (stop-loss/take-profit/trailing) + touched fills

## [0.4.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-core-v0.3.0...yuzu-core-v0.4.0) - 2026-07-10

### Added

- *(core)* vol_target op — portfolio volatility targeting
- *(core)* carry positions across walk-forward window seams ([#21](https://github.com/imgarylai/citrusinvest/pull/21))
- *(core)* factor report + event study research API ([#45](https://github.com/imgarylai/citrusinvest/pull/45))
- *(core)* cap_industry op — per-industry gross weight cap
- *(core)* expose per-trade fill prices and side in the report

## [0.3.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-core-v0.2.0...yuzu-core-v0.3.0) - 2026-07-10

### Added

- *(ops)* Bollinger, MACD, Donchian + rolling_min surface
- *(report)* lookback returns, tail-risk, and drawdown-shape metrics
- *(ops)* in_sector membership mask from industry map
- *(report)* live_performance_start post-go-live segment metrics
- *(ops)* cross-section winsorize, zscore, bucket, demean
- *(lemon)* expose exit_when and quantile_row on the DSL surface
- *(yuzu-py)* Python bindings via PyO3 (closes #25)
- *(yuzu-core)* square-root market-impact cost (closes #19)

### Other

- Starter-tier data gaps vs citrusinvest features
- rustfmt for exit_when / quantile_row
- add input data layout for bring-your-own panels
- *(lemon)* rename crates/lemon directory to crates/lemon-lang

## [0.2.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-core-v0.1.1...yuzu-core-v0.2.0) - 2026-07-08

### Added

- *(lemon)* editor language services, LSP server, and VS Code extension
- *(yuzu-core)* YE (year-end) rebalance frequency
- normalize_row op — explicit portfolio weights from a raw signal
- *(lemon)* `not` prefix operator
- *(yuzu-core)* block-bootstrap confidence bands for Sharpe/CAGR/MDD
- *(yuzu-core)* monthly/yearly return tables and rolling metrics in Report
- *(yuzu-core)* Panel::slice_dates for windowed backtests
- *(yuzu-core)* benchmark comparison — alpha/beta/IR and rebased curve
- *(yuzu-core)* delisting detection with forced exit and haircut
- *(yuzu-core)* volume-participation liquidity cap on weights
- *(yuzu-core)* slippage cost on turnover and trade returns

### Other

- *(yuzu-core)* add runnable basic_backtest example
- crates.io keywords/categories, MSRV 1.86, CI + docs badges

## [0.1.1](https://github.com/imgarylai/citrusinvest/compare/yuzu-core-v0.1.0...yuzu-core-v0.1.1) - 2026-07-08

### Other

- add per-crate READMEs and crates.io badges
- release v0.1.0

## [0.1.0](https://github.com/imgarylai/citrusinvest/releases/tag/yuzu-core-v0.1.0) - 2026-07-08

### Other

- add CI, release-plz, and Pages docs; crates.io metadata
- rustfmt + clippy-clean the workspace
- make repo standalone — refresh docs, rebrand, wasm scripts to dist/OUT
- initial import of the citrusinvest engine
