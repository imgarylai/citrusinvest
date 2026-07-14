# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.0](https://github.com/citrusquant/citrusquant/compare/pomelo-eodhd-v0.11.0...pomelo-eodhd-v0.12.0) - 2026-07-14

### Other

- give each crate its own README

## [0.11.0](https://github.com/citrusquant/citrusquant/compare/pomelo-eodhd-v0.10.0...pomelo-eodhd-v0.11.0) - 2026-07-14

### Added

- *(pomelo-finnhub)* crate skeleton + yuzu-cli finnhub-sync ([#225](https://github.com/citrusquant/citrusquant/pull/225))
- *(pomelo-alpha-vantage)* snapshot factors + alpha-vantage-data-source docs ([#218](https://github.com/citrusquant/citrusquant/pull/218))
- *(pomelo-alpha-vantage)* crate skeleton + yuzu-cli av-sync ([#213](https://github.com/citrusquant/citrusquant/pull/213)) ([#219](https://github.com/citrusquant/citrusquant/pull/219))

### Other

- extract pomelo-http; move pure factor math to pomelo-data ([#211](https://github.com/citrusquant/citrusquant/pull/211))
- re-audit multi-source gaps after EODHD path ([#186](https://github.com/citrusquant/citrusquant/pull/186))

## [0.10.0](https://github.com/citrusquant/citrusquant/compare/pomelo-eodhd-v0.9.0...pomelo-eodhd-v0.10.0) - 2026-07-13

### Added

- *(pomelo-eodhd)* snapshot factors + eodhd-data-source docs ([#198](https://github.com/citrusquant/citrusquant/pull/198))
- *(pomelo-eodhd)* SPX PIT panel + eodhd-symbols screener ([#197](https://github.com/citrusquant/citrusquant/pull/197))
- *(pomelo-eodhd)* densify annual fundamentals + report_event ([#196](https://github.com/citrusquant/citrusquant/pull/196))
- *(pomelo-eodhd)* industry map + delisted universe ([#195](https://github.com/citrusquant/citrusquant/pull/195))
- *(pomelo-eodhd)* EOD prices → data-layout with adj OHLC ([#194](https://github.com/citrusquant/citrusquant/pull/194))

### Other

- *(pomelo-eodhd)* cover snapshot/fundamentals sync paths
- *(pomelo-eodhd)* raise snapshot/factor coverage
- *(pomelo-eodhd)* cover index fetch/write and screener edges
- *(pomelo-eodhd)* raise coverage for http/price/sync paths
