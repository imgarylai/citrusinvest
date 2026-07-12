# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/citrusquant/citrusquant/compare/pomelo-data-v0.5.1...pomelo-data-v0.6.0) - 2026-07-12

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

> Renamed from `yuzu-data`; entries below predate the rename and keep their original `yuzu-data-v*` tags.

## [0.5.1](https://github.com/citrusquant/citrusquant/compare/yuzu-data-v0.5.0...yuzu-data-v0.5.1) - 2026-07-10

### Other

- *(yuzu-data)* share date_to_i32 / i32_to_date helpers
- citrusinvest -> citrusquant (brand, URLs, org/repo, site domain)
- release v0.5.0

## [0.5.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-data-v0.4.0...yuzu-data-v0.5.0) - 2026-07-10

### Other

- release v0.5.0

## [0.3.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-data-v0.2.0...yuzu-data-v0.3.0) - 2026-07-10

### Added

- *(yuzu-py)* Python bindings via PyO3 (closes #25)

### Other

- Starter-tier data gaps vs citrusinvest features
- add input data layout for bring-your-own panels

## [0.2.0](https://github.com/imgarylai/citrusinvest/compare/yuzu-data-v0.1.1...yuzu-data-v0.2.0) - 2026-07-08

### Added

- *(lemon)* editor language services, LSP server, and VS Code extension
- *(yuzu-data)* read Parquet and plain CSV inputs

### Other

- crates.io keywords/categories, MSRV 1.86, CI + docs badges

## [0.1.1](https://github.com/imgarylai/citrusinvest/compare/yuzu-data-v0.1.0...yuzu-data-v0.1.1) - 2026-07-08

### Other

- add per-crate READMEs and crates.io badges
- release v0.1.0

## [0.1.0](https://github.com/imgarylai/citrusinvest/releases/tag/yuzu-data-v0.1.0) - 2026-07-08

### Other

- add CI, release-plz, and Pages docs; crates.io metadata
- rustfmt + clippy-clean the workspace
- make repo standalone — refresh docs, rebrand, wasm scripts to dist/OUT
- initial import of the citrusinvest engine
