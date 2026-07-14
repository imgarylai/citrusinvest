# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.1](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.12.0...yuzu-research-v0.12.1) - 2026-07-14

### Added

- named point-in-time index universe (`#! index:` / envelope symbols_hint) ([#256](https://github.com/citrusquant/citrusquant/pull/256))

## [0.12.0](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.11.0...yuzu-research-v0.12.0) - 2026-07-14

### Added

- load the series a strategy actually references (CLI path) ([#251](https://github.com/citrusquant/citrusquant/pull/251))
- [**breaking**] scope backtests to an explicit symbols universe ([#250](https://github.com/citrusquant/citrusquant/pull/250))

### Other

- give each crate its own README

## [0.11.0](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.10.0...yuzu-research-v0.11.0) - 2026-07-14

### Added

- *(pomelo-finnhub)* crate skeleton + yuzu-cli finnhub-sync ([#225](https://github.com/citrusquant/citrusquant/pull/225))
- *(pomelo-alpha-vantage)* snapshot factors + alpha-vantage-data-source docs ([#218](https://github.com/citrusquant/citrusquant/pull/218))
- *(pomelo-alpha-vantage)* crate skeleton + yuzu-cli av-sync ([#213](https://github.com/citrusquant/citrusquant/pull/213)) ([#219](https://github.com/citrusquant/citrusquant/pull/219))

### Other

- re-audit multi-source gaps after EODHD path ([#186](https://github.com/citrusquant/citrusquant/pull/186))

## [0.10.0](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.9.0...yuzu-research-v0.10.0) - 2026-07-13

### Added

- *(pomelo-eodhd)* snapshot factors + eodhd-data-source docs ([#198](https://github.com/citrusquant/citrusquant/pull/198))
- *(pomelo-eodhd)* crate skeleton + yuzu-cli eodhd-sync ([#193](https://github.com/citrusquant/citrusquant/pull/193))

### Other

- add WIP data-sources guide for non-FMP assemble paths

## [0.9.0](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.8.2...yuzu-research-v0.9.0) - 2026-07-13

### Fixed

- correct doc drift (rise/fall, op counts, crate publish list, Parquet)

## [0.8.1](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.7.0...yuzu-research-v0.8.1) - 2026-07-12

### Other

- release v0.7.1

## [0.7.0](https://github.com/citrusquant/citrusquant/compare/yuzu-research-v0.6.0...yuzu-research-v0.7.0) - 2026-07-12

### Added

- *(yuzu-py)* Python bindings via PyO3 (closes #25)
- *(lemon)* editor language services, LSP server, and VS Code extension

### Other

- *(yuzu-research)* extract multi-run research orchestration from the CLI
- *(audit)* move data-audit into a pomelo-audit crate; CLI just calls it
- address #139 review — three-family docs + restore CHANGELOG tags
- *(data)* [**breaking**] rename yuzu-data → pomelo-data, yuzu-source-s3 → pomelo-s3
- *(data)* extract pomelo-fmp crate (FMP sync + factor formulas)
- citrusinvest -> citrusquant (brand, URLs, org/repo, site domain)
- Starter-tier data gaps vs citrusinvest features
- add input data layout for bring-your-own panels
- crates.io keywords/categories, MSRV 1.86, CI + docs badges
- add per-crate READMEs and crates.io badges
- make repo standalone — refresh docs, rebrand, wasm scripts to dist/OUT
- initial import of the citrusinvest engine
