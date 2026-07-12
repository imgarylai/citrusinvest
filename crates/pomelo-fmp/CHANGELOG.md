# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/citrusquant/citrusquant/compare/pomelo-fmp-v0.5.1...pomelo-fmp-v0.6.0) - 2026-07-12

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
