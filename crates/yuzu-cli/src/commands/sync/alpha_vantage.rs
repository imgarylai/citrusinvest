use std::path::PathBuf;

use clap::Args;

#[derive(Args)]
pub(crate) struct AvSyncArgs {
    /// Alpha Vantage API key (kept local). Falls back to
    /// $ALPHA_VANTAGE_API_KEY or $ALPHAVANTAGE_API_KEY if unset.
    #[arg(long)]
    api_key: Option<String>,
    /// Output data root — local path or `s3://bucket[/prefix]`.
    #[arg(long)]
    out: String,
    /// Comma-separated tickers (`AAPL` or `TSCO.LON`). Or use --symbols-file.
    #[arg(long, value_delimiter = ',')]
    symbols: Vec<String>,
    /// Read symbols from a file (one per line, or comma-separated; `#` comments).
    #[arg(long)]
    symbols_file: Option<PathBuf>,
    /// Default exchange hint for bare tickers (US equities stay bare on AV).
    #[arg(long, default_value = yuzu_cli::alpha_vantage::DEFAULT_EXCHANGE)]
    exchange: String,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 20991231)]
    to: i32,
    /// Also densify annual IS/BS → fundamentals/{SYM}.csv.gz
    /// (period-end visibility; pe/ps/pb/market_cap left NaN historically).
    #[arg(long)]
    include_fundamentals: bool,
    /// Also fetch sector map from OVERVIEW → tracked/universe.csv.gz.
    #[arg(long)]
    include_industry: bool,
    /// Also union LISTING_STATUS delisted names into the sync list
    /// (survivorship-honest price files when those bars still exist).
    #[arg(long)]
    include_delisted: bool,
    /// Best-effort snapshot panels (analyst_upside_pct, consensus_rating,
    /// fcf_yield, pe_industry_pctile) → panels/. Current-as-of; no piotroski/altman.
    #[arg(long)]
    include_snapshot_factors: bool,
    /// Max requests per minute (0 = no throttle).
    #[arg(long, default_value_t = 0)]
    rate_limit: u32,
    /// Retries on 429 / 5xx / transport errors.
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
    /// Merge fetched rows into existing files instead of overwriting.
    #[arg(long)]
    append: bool,
    /// Skip symbols that already have a price file.
    #[arg(long)]
    resume: bool,
}

#[derive(Args)]
pub(crate) struct AvSymbolsArgs {
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    out: PathBuf,
    /// Exchange filter (`NYSE`, `NASDAQ`, …). Pass `all` for every exchange.
    #[arg(long, default_value = "all")]
    exchange: String,
    /// Asset type filter (default: Stock). Pass `all` for ETFs/funds too.
    #[arg(long, default_value = "Stock")]
    asset_type: String,
    /// Cap the number of symbols returned (after sort).
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, default_value_t = 0)]
    rate_limit: u32,
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
}

pub(crate) fn sync(args: AvSyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let AvSyncArgs {
        api_key,
        out,
        symbols,
        symbols_file,
        exchange,
        from,
        to,
        include_fundamentals,
        include_industry,
        include_delisted,
        include_snapshot_factors,
        rate_limit,
        max_retries,
        append,
        resume,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("ALPHA_VANTAGE_API_KEY").ok())
        .or_else(|| std::env::var("ALPHAVANTAGE_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set ALPHA_VANTAGE_API_KEY (or ALPHAVANTAGE_API_KEY)")?;
    let mut explicit: Vec<String> = symbols
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(path) = symbols_file {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        explicit.extend(yuzu_cli::alpha_vantage::parse_symbols_list(&text));
    }
    let mut seen = std::collections::HashSet::new();
    explicit.retain(|s| seen.insert(s.clone()));

    let mode = if resume {
        yuzu_cli::alpha_vantage::WriteMode::Resume
    } else if append {
        yuzu_cli::alpha_vantage::WriteMode::Append
    } else {
        yuzu_cli::alpha_vantage::WriteMode::Overwrite
    };
    let cfg = yuzu_cli::alpha_vantage::SyncConfig {
        from,
        to,
        default_exchange: exchange,
        include_fundamentals,
        include_industry,
        include_snapshot_factors,
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        mode,
    };
    let client = yuzu_cli::alpha_vantage::UreqClient::new();

    if include_delisted {
        eprintln!("fetching delisted universe from Alpha Vantage LISTING_STATUS…");
        let delisted = yuzu_cli::alpha_vantage::fetch_delisted(&client, &api_key, &cfg)?;
        let before = explicit.len();
        for d in delisted {
            if seen.insert(d.symbol.clone()) {
                explicit.push(d.symbol);
            }
        }
        eprintln!("delisted: +{} names unioned in", explicit.len() - before);
    }
    if explicit.is_empty() {
        return Err("provide --symbols and/or --symbols-file, or --include-delisted".into());
    }

    let out_store = pomelo_s3::OutStore::parse(&out)?;
    let summary =
        yuzu_cli::alpha_vantage::sync_into(&client, &api_key, &explicit, &out_store, &cfg)?;
    eprintln!(
        "done: {} written, {} skipped, {} filtered, {} price rows, {} fundamentals, industry={}, snapshot_panels={}, {} failures",
        summary.symbols_written,
        summary.symbols_skipped,
        summary.symbols_filtered,
        summary.price_rows,
        summary.fundamentals_written,
        summary.industry_written,
        summary.snapshot_factor_panels,
        summary.failures.len(),
    );
    for (sym, err) in &summary.failures {
        eprintln!("  FAILED {sym}: {err}");
    }
    if summary.symbols_written == 0 {
        return Err("no symbols were written".into());
    }
    Ok(())
}

pub(crate) fn symbols(args: AvSymbolsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let AvSymbolsArgs {
        api_key,
        out,
        exchange,
        asset_type,
        limit,
        rate_limit,
        max_retries,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("ALPHA_VANTAGE_API_KEY").ok())
        .or_else(|| std::env::var("ALPHAVANTAGE_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set ALPHA_VANTAGE_API_KEY (or ALPHAVANTAGE_API_KEY)")?;
    let cfg = yuzu_cli::alpha_vantage::SyncConfig {
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    let filter = yuzu_cli::alpha_vantage::SymbolFilter {
        exchange,
        asset_type,
        limit,
    };
    let client = yuzu_cli::alpha_vantage::UreqClient::new();
    eprintln!(
        "building symbol universe from Alpha Vantage LISTING_STATUS (active)… \
         (not a market-cap screener; no index PIT)"
    );
    let syms = yuzu_cli::alpha_vantage::build_symbol_list(&client, &api_key, &cfg, &filter)?;
    if syms.is_empty() {
        return Err("listing returned no symbols (loosen --exchange / --asset-type?)".into());
    }
    let mut body = String::from(
        "# symbols built by `yuzu-cli av-symbols` (LISTING_STATUS active)\n\
         # no index membership — AV cannot write honest panels/in_sp500\n",
    );
    for s in &syms {
        body.push_str(s);
        body.push('\n');
    }
    std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
    eprintln!("wrote {} symbols to {}", syms.len(), out.display());
    Ok(())
}
