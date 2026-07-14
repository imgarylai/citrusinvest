# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.10.0...pomelo-s3-v0.11.0) - 2026-07-14

### Added

- *(pomelo-finnhub)* crate skeleton + yuzu-cli finnhub-sync ([#225](https://github.com/citrusquant/citrusquant/pull/225))
- *(pomelo-alpha-vantage)* snapshot factors + alpha-vantage-data-source docs ([#218](https://github.com/citrusquant/citrusquant/pull/218))
- *(pomelo-alpha-vantage)* crate skeleton + yuzu-cli av-sync ([#213](https://github.com/citrusquant/citrusquant/pull/213)) ([#219](https://github.com/citrusquant/citrusquant/pull/219))

### Other

- re-audit multi-source gaps after EODHD path ([#186](https://github.com/citrusquant/citrusquant/pull/186))

## [0.10.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.9.0...pomelo-s3-v0.10.0) - 2026-07-13

### Added

- *(pomelo-eodhd)* snapshot factors + eodhd-data-source docs ([#198](https://github.com/citrusquant/citrusquant/pull/198))
- *(pomelo-eodhd)* crate skeleton + yuzu-cli eodhd-sync ([#193](https://github.com/citrusquant/citrusquant/pull/193))

### Other

- add WIP data-sources guide for non-FMP assemble paths

## [0.9.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.8.2...pomelo-s3-v0.9.0) - 2026-07-13

### Fixed

- correct doc drift (rise/fall, op counts, crate publish list, Parquet)

## [0.8.1](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.7.0...pomelo-s3-v0.8.1) - 2026-07-12

### Added

- *(audit)* audit an S3/R2 tree via a storage-agnostic ObjectLister

### Other

- release v0.7.1

## [0.8.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.7.0...pomelo-s3-v0.8.0) - 2026-07-12

### Added

- *(audit)* audit an S3/R2 tree via a storage-agnostic ObjectLister

## [0.7.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.6.0...pomelo-s3-v0.7.0) - 2026-07-12

### Added

- *(fmp-sync)* S3 credential chain (R2 + AWS IAM), session-token support

### Other

- *(yuzu-research)* extract multi-run research orchestration from the CLI
- relocate library-grade logic from yuzu-cli to pomelo crates
- *(audit)* move data-audit into a pomelo-audit crate; CLI just calls it

## [0.6.0](https://github.com/citrusquant/citrusquant/compare/pomelo-s3-v0.5.1...pomelo-s3-v0.6.0) - 2026-07-12

### Added

- *(yuzu-py)* Python bindings via PyO3 (closes #25)
- *(lemon)* editor language services, LSP server, and VS Code extension

### Other

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

> Renamed from `yuzu-source-s3`; entries below predate the rename and keep their original `yuzu-source-s3-v*` tags.

## [0.5.1](https://github.com/citrusquant/citrusquant/compare/yuzu-source-s3-v0.5.0...yuzu-source-s3-v0.5.1) - 2026-07-10

### Other

- citrusinvest -> citrusquant (brand, URLs, org/repo, site domain)
- release v0.5.0

## [0.5.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-source-s3-v0.4.0...yuzu-source-s3-v0.5.0) - 2026-07-10

### Other

- release v0.5.0

## [0.3.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-source-s3-v0.2.0...yuzu-source-s3-v0.3.0) - 2026-07-10

### Added

- *(yuzu-py)* Python bindings via PyO3 (closes #25)

### Other

- Starter-tier data gaps vs citrusinvest features
- add input data layout for bring-your-own panels

## [0.2.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-source-s3-v0.1.1...yuzu-source-s3-v0.2.0) - 2026-07-08

### Added

- *(lemon)* editor language services, LSP server, and VS Code extension

### Other

- enforce 95% coverage gate and cover untested library paths
- crates.io keywords/categories, MSRV 1.86, CI + docs badges

## [0.1.1](https://github.com/imgarylai/citrusinvest/compare/yuzu-source-s3-v0.1.0...yuzu-source-s3-v0.1.1) - 2026-07-08

### Other

- add per-crate READMEs and crates.io badges
- release v0.1.0

## [0.1.0](https://github.com/imgarylai/citrusinvest/releases/tag/yuzu-source-s3-v0.1.0) - 2026-07-08

### Other

- add CI, release-plz, and Pages docs; crates.io metadata
- rustfmt + clippy-clean the workspace
- make repo standalone — refresh docs, rebrand, wasm scripts to dist/OUT
- initial import of the citrusinvest engine
