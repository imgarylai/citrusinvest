# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1](https://github.com/citrusquant/citrusquant/compare/yuzu-core-v0.5.0...yuzu-core-v0.5.1) - 2026-07-10

### Other

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
