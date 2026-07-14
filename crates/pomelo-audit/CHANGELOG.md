# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.1](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.12.0...pomelo-audit-v0.12.1) - 2026-07-14

### Other

- release v0.12.1

## [0.12.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.11.0...pomelo-audit-v0.12.0) - 2026-07-14

### Other

- give each crate its own README

## [0.11.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.10.0...pomelo-audit-v0.11.0) - 2026-07-14

### Added

- *(pomelo-finnhub)* crate skeleton + yuzu-cli finnhub-sync ([#225](https://github.com/citrusquant/citrusquant/pull/225))
- *(pomelo-alpha-vantage)* snapshot factors + alpha-vantage-data-source docs ([#218](https://github.com/citrusquant/citrusquant/pull/218))
- *(pomelo-alpha-vantage)* crate skeleton + yuzu-cli av-sync ([#213](https://github.com/citrusquant/citrusquant/pull/213)) ([#219](https://github.com/citrusquant/citrusquant/pull/219))

### Other

- re-audit multi-source gaps after EODHD path ([#186](https://github.com/citrusquant/citrusquant/pull/186))

## [0.10.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.9.0...pomelo-audit-v0.10.0) - 2026-07-13

### Added

- *(pomelo-eodhd)* snapshot factors + eodhd-data-source docs ([#198](https://github.com/citrusquant/citrusquant/pull/198))
- *(pomelo-eodhd)* crate skeleton + yuzu-cli eodhd-sync ([#193](https://github.com/citrusquant/citrusquant/pull/193))

### Other

- add WIP data-sources guide for non-FMP assemble paths

## [0.9.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.8.2...pomelo-audit-v0.9.0) - 2026-07-13

### Fixed

- correct doc drift (rise/fall, op counts, crate publish list, Parquet)

## [0.8.2](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.8.1...pomelo-audit-v0.8.2) - 2026-07-12

### Other

- *(audit)* split pomelo-audit/src/lib.rs into modules

## [0.8.1](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.7.0...pomelo-audit-v0.8.1) - 2026-07-12

### Added

- *(audit)* audit an S3/R2 tree via a storage-agnostic ObjectLister

### Fixed

- *(pomelo-audit)* bump pomelo-s3 dev-dependency to 0.7.0

### Other

- release v0.7.1

## [0.8.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.7.0...pomelo-audit-v0.8.0) - 2026-07-12

### Added

- *(audit)* audit an S3/R2 tree via a storage-agnostic ObjectLister

### Fixed

- *(pomelo-audit)* bump pomelo-s3 dev-dependency to 0.7.0

## [0.7.0](https://github.com/citrusquant/citrusquant/compare/pomelo-audit-v0.6.0...pomelo-audit-v0.7.0) - 2026-07-12

### Other

- *(yuzu-research)* extract multi-run research orchestration from the CLI
- relocate library-grade logic from yuzu-cli to pomelo crates
