use std::path::PathBuf;

use clap::Args;

use crate::commands::IndexArg;

#[derive(Args)]
pub(crate) struct FmpSyncArgs {
    /// FMP API key (kept local). Falls back to $FMP_API_KEY if unset.
    #[arg(long)]
    api_key: Option<String>,
    /// Output data root — a local path, or an `s3://bucket[/prefix]` URL to
    /// write directly to S3/R2. Credentials resolve from the env, trying the
    /// `S3_*` vars first (R2 / static keys) then `AWS_*` (AWS S3 + IAM role,
    /// incl. `AWS_SESSION_TOKEN`) — see docs/fmp-data-source.md. Both targets
    /// produce a byte-identical tree. (`--index` needs a local path.)
    #[arg(long)]
    out: String,
    /// Comma-separated tickers, e.g. AAPL,MSFT,GOOGL. Or use --symbols-file
    /// to sync a prebuilt list, or --all-symbols for the whole universe.
    #[arg(long, value_delimiter = ',')]
    symbols: Vec<String>,
    /// Read the symbol universe from a file (one ticker per line, or a
    /// `symbol,...` CSV) — e.g. one built by `yuzu-cli fmp-symbols`.
    /// Mutually exclusive with --symbols / --all-symbols.
    #[arg(long)]
    symbols_file: Option<PathBuf>,
    /// Sync the whole screened universe (FMP screener) instead of an
    /// explicit list — the exchanges in --exchange (default US: NASDAQ,NYSE,
    /// AMEX), honoring --min-market-cap / --include-etf. Large; combine with
    /// --rate-limit / --resume. Mutually exclusive with --symbols.
    #[arg(long)]
    all_symbols: bool,
    /// Reconstruct a point-in-time index universe: sync every name that was
    /// ever a member over [from,to] (survivorship-honest, incl. names that
    /// later left), and write a `in_<index>` 0/1 membership panel to
    /// panels/. Backtest with `signal * in_sp500`. Index-scoped and
    /// degrades for very old dates (#125). Mutually exclusive with the other
    /// universe sources.
    #[arg(long, value_enum)]
    index: Option<IndexArg>,
    /// Exchanges for --all-symbols (comma-separated FMP codes). Default the
    /// three US majors; pass `all` for every exchange.
    #[arg(long, default_value = yuzu_cli::fmp::US_EXCHANGES)]
    exchange: String,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 20991231)]
    to: i32,
    /// Also fetch annual fundamentals (best-effort; see #51 / #53).
    #[arg(long)]
    include_fundamentals: bool,
    /// Also fetch each symbol's sector → tracked/universe.csv.gz.
    #[arg(long)]
    include_industry: bool,
    /// Also compute snapshot-factor panels (piotroski_score, altman_z,
    /// fcf_yield, analyst_upside_pct, consensus_rating) → panels/{name}.csv.gz.
    /// Current-snapshot factors for universe screening; extra FMP requests
    /// per symbol (see docs/fmp-data-source.md).
    #[arg(long)]
    include_snapshot_factors: bool,
    /// Keep ETFs and mutual/closed-end funds. By default only individual
    /// stocks are synced (non-stocks are classified via the profile
    /// endpoint and skipped).
    #[arg(long)]
    include_etf: bool,
    /// Also union FMP's delisted-companies universe (filtered by --exchange)
    /// into the symbol list, so dead names are synced too — their price
    /// files simply end at the delisting date, and the engine's delist_after
    /// forced-exit handles them. Removes survivorship bias at the data layer
    /// (#124 / #26). Note: delisted rows carry no market cap, so
    /// --min-market-cap does not filter them.
    #[arg(long)]
    include_delisted: bool,
    /// Skip symbols whose company market cap is below this, in USD (0 = off).
    /// Accepts unit suffixes: 1b, 500m, 10k, 2.5t (or a plain number / 1e9).
    #[arg(long, default_value = "0", value_parser = yuzu_cli::fmp::parse_market_cap)]
    min_market_cap: f64,
    /// Max requests per minute (0 = no throttle). Match your FMP plan's
    /// rate limit (Starter-class keys are commonly ~300/min).
    #[arg(long, default_value_t = 300)]
    rate_limit: u32,
    /// Retries per request on 429 / 5xx / transport errors.
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
    /// Merge fetched rows into existing files instead of overwriting
    /// (extend an existing tree's history).
    #[arg(long)]
    append: bool,
    /// Skip symbols that already have a price file (resume an interrupted
    /// run). Takes precedence over --append.
    #[arg(long)]
    resume: bool,
}

#[derive(Args)]
pub(crate) struct FmpSymbolsArgs {
    /// FMP API key (kept local). Falls back to $FMP_API_KEY if unset.
    #[arg(long)]
    api_key: Option<String>,
    /// Output file for the symbol list (one ticker per line).
    #[arg(long)]
    out: PathBuf,
    /// Only symbols at/above this company market cap, in USD (0 = no floor).
    /// Accepts unit suffixes: 1b, 500m, 10k, 2.5t (or a plain number / 1e9).
    #[arg(long, default_value = "0", value_parser = yuzu_cli::fmp::parse_market_cap)]
    min_market_cap: f64,
    /// Restrict to one or more exchanges (comma-separated FMP codes).
    /// Default the three US majors (NASDAQ,NYSE,AMEX); pass `all` for every
    /// exchange.
    #[arg(long, default_value = yuzu_cli::fmp::US_EXCHANGES)]
    exchange: String,
    /// Include ETFs and funds (default: stocks only).
    #[arg(long)]
    include_etf: bool,
    /// Also append FMP's delisted-companies universe (filtered by
    /// --exchange) to the list, so a whole-market backtest is
    /// survivorship-honest (#124 / #26). Delisted rows carry no market cap,
    /// so --min-market-cap does not filter them.
    #[arg(long)]
    include_delisted: bool,
    /// Cap the number of symbols returned (default: the API's).
    #[arg(long)]
    limit: Option<usize>,
    /// Max requests per minute (0 = no throttle).
    #[arg(long, default_value_t = 300)]
    rate_limit: u32,
    /// Retries on 429 / 5xx / transport errors.
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
}

pub(crate) fn sync(args: FmpSyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let FmpSyncArgs {
        api_key,
        out,
        symbols,
        symbols_file,
        all_symbols,
        index,
        exchange,
        from,
        to,
        include_fundamentals,
        include_industry,
        include_snapshot_factors,
        include_etf,
        include_delisted,
        min_market_cap,
        rate_limit,
        max_retries,
        append,
        resume,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("FMP_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set FMP_API_KEY")?;
    let explicit: Vec<String> = symbols
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Exactly one universe source: --symbols, --symbols-file, --all-symbols, --index.
    let sources = [
        !explicit.is_empty(),
        symbols_file.is_some(),
        all_symbols,
        index.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if sources > 1 {
        return Err("choose one of --symbols, --symbols-file, --all-symbols, or --index".into());
    }
    if sources == 0 {
        return Err(
            "provide --symbols AAPL,MSFT,..., --symbols-file <path>, --all-symbols, or --index <sp500|nasdaq|dowjones>"
                .into(),
        );
    }
    let mode = if resume {
        yuzu_cli::fmp::WriteMode::Resume
    } else if append {
        yuzu_cli::fmp::WriteMode::Append
    } else {
        yuzu_cli::fmp::WriteMode::Overwrite
    };
    let cfg = yuzu_cli::fmp::SyncConfig {
        from,
        to,
        include_fundamentals,
        include_industry,
        include_snapshot_factors,
        skip_non_stocks: !include_etf,
        min_market_cap,
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        mode,
    };
    let client = yuzu_cli::fmp::UreqClient::new();
    // For --index, fetch the reconstructor up front: its ever-members are
    // the sync universe, and it's reused after the sync to write the
    // membership panel over the resulting price calendar.
    let membership = match index {
        Some(idx) => Some(yuzu_cli::fmp::IndexMembership::fetch(
            &client,
            &api_key,
            idx.into(),
            &cfg,
        )?),
        None => None,
    };
    let mut symbols = if let Some(m) = &membership {
        eprintln!(
            "reconstructing {} point-in-time membership…",
            m.series_name()
        );
        let ever = m.ever_members(from, to);
        eprintln!(
            "index universe: {} ever-members over [{from}, {to}]",
            ever.len()
        );
        ever
    } else if all_symbols {
        let filter = yuzu_cli::fmp::SymbolFilter {
            min_market_cap,
            exchange: Some(exchange.clone()),
            include_etf,
            limit: None,
        };
        eprintln!("building symbol universe from FMP screener…");
        let all = yuzu_cli::fmp::build_symbol_list(&client, &api_key, &cfg, &filter)?;
        eprintln!("universe: {} symbols", all.len());
        all
    } else if let Some(path) = symbols_file {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let syms = yuzu_cli::fmp::parse_symbols_list(&text);
        if syms.is_empty() {
            return Err(format!("no symbols in {}", path.display()).into());
        }
        eprintln!("universe: {} symbols from {}", syms.len(), path.display());
        syms
    } else {
        explicit
    };
    if include_delisted {
        eprintln!("fetching delisted universe from FMP…");
        let delisted = yuzu_cli::fmp::fetch_delisted(&client, &api_key, &cfg, Some(&exchange))?;
        let mut seen: std::collections::HashSet<String> = symbols.iter().cloned().collect();
        let before = symbols.len();
        for d in delisted {
            if seen.insert(d.symbol.clone()) {
                symbols.push(d.symbol);
            }
        }
        eprintln!("delisted: +{} names unioned in", symbols.len() - before);
    }
    let out_store = pomelo_s3::OutStore::parse(&out)?;
    if membership.is_some() && out_store.is_s3() {
        return Err("--index requires a local --out (the membership panel write reads a local trading calendar); sync to a local path, or omit --index".into());
    }
    let summary = yuzu_cli::fmp::sync_into(&client, &api_key, &symbols, &out_store, &cfg)?;
    eprintln!(
        "done: {} written, {} skipped, {} filtered, {} price rows, {} fundamentals, {} failures",
        summary.symbols_written,
        summary.symbols_skipped,
        summary.symbols_filtered,
        summary.price_rows,
        summary.fundamentals_written,
        summary.failures.len(),
    );
    for (sym, err) in &summary.failures {
        eprintln!("  FAILED {sym}: {err}");
    }
    if summary.symbols_written == 0 {
        return Err("no symbols were written".into());
    }
    // Now that prices exist (a trading calendar), write the PIT
    // membership panel over exactly those days.
    if let Some(m) = &membership {
        // Guarded above: reaching here means `out` is a local path.
        let (days, cols) =
            yuzu_cli::fmp::write_index_membership(std::path::Path::new(&out), m, from, to)?;
        eprintln!(
            "wrote panels/{}.csv.gz: {days} days × {cols} symbols (mask with mask(signal, {}))",
            m.series_name(),
            m.series_name()
        );
    }
    Ok(())
}

pub(crate) fn symbols(args: FmpSymbolsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let FmpSymbolsArgs {
        api_key,
        out,
        min_market_cap,
        exchange,
        include_etf,
        include_delisted,
        limit,
        rate_limit,
        max_retries,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("FMP_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set FMP_API_KEY")?;
    let cfg = yuzu_cli::fmp::SyncConfig {
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    let filter = yuzu_cli::fmp::SymbolFilter {
        min_market_cap,
        exchange: Some(exchange.clone()),
        include_etf,
        limit,
    };
    let client = yuzu_cli::fmp::UreqClient::new();
    eprintln!("building symbol universe from FMP screener…");
    let mut syms = yuzu_cli::fmp::build_symbol_list(&client, &api_key, &cfg, &filter)?;
    if syms.is_empty() {
        return Err("screener returned no symbols (loosen the filters?)".into());
    }
    if include_delisted {
        eprintln!("fetching delisted universe from FMP…");
        let delisted = yuzu_cli::fmp::fetch_delisted(&client, &api_key, &cfg, Some(&exchange))?;
        let mut seen: std::collections::HashSet<String> = syms.iter().cloned().collect();
        let before = syms.len();
        for d in delisted {
            if seen.insert(d.symbol.clone()) {
                syms.push(d.symbol);
            }
        }
        syms.sort();
        eprintln!("delisted: +{} names appended", syms.len() - before);
    }
    let mut body = String::from("# symbols built by `yuzu-cli fmp-symbols`\n");
    for s in &syms {
        body.push_str(s);
        body.push('\n');
    }
    std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
    eprintln!("wrote {} symbols to {}", syms.len(), out.display());
    Ok(())
}
