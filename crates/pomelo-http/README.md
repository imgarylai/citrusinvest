# pomelo-http

[![crates.io](https://img.shields.io/crates/v/pomelo-http)](https://crates.io/crates/pomelo-http)

Shared HTTP plumbing for the `pomelo-*` bring-your-own-key market-data sync
crates — part of [citrusquant](https://github.com/citrusquant/citrusquant).

Every vendor adapter (`pomelo-fmp`, `pomelo-eodhd`, `pomelo-alpha-vantage`,
`pomelo-finnhub`) needs the same primitives, so they live here once:

- a mockable `HttpClient`,
- a `Fetcher` that adds rate-limit throttle + bounded exponential-backoff retry,
- token `redact`ion for logs, and
- a `WriteMode`.

**No vendor logic.** JSON field maps, densify formulas, symbol-suffix rules, and
rating mappers stay in each vendor crate — the `Fetcher` reads its throttle/retry
knobs through the `RetrySettings` trait, so no vendor `SyncConfig` type leaks in.

See the [data sources overview](https://github.com/citrusquant/citrusquant/blob/main/docs/data-sources.md).

## License

MIT
