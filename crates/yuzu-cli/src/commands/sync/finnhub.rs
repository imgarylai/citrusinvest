use std::path::PathBuf;

use clap::Args;

use crate::commands::FinnhubIndexArg;

#[derive(Args)]
pub(crate) struct FinnhubSyncArgs {
    /// Finnhub API key (kept local). Falls back to $FINNHUB_API_KEY if unset.
    #[arg(long)]
    api_key: Option<String>,
    /// Output data root — local path or `s3://bucket[/prefix]`.
    #[arg(long)]
    out: String,
    /// Comma-separated tickers (`AAPL` or `TSCO.L`). Or use --symbols-file.
    #[arg(long, value_delimiter = ',')]
    symbols: Vec<String>,
    /// Read symbols from a file (one per line, or comma-separated; `#` comments).
    #[arg(long)]
    symbols_file: Option<PathBuf>,
    /// Reconstruct a point-in-time index universe: sync every name that was
    /// an S&P 500 member over [from,to] and write `panels/in_sp500.csv.gz`
    /// (local --out only). Combines with --symbols / --symbols-file.
    #[arg(long, value_enum)]
    index: Option<FinnhubIndexArg>,
    /// Default exchange hint for bare tickers.
    #[arg(long, default_value = yuzu_cli::finnhub::DEFAULT_EXCHANGE)]
    exchange: String,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 20991231)]
    to: i32,
    /// Also densify annual `/stock/financials-reported` → fundamentals/{SYM}.csv.gz.
    #[arg(long)]
    include_fundamentals: bool,
    /// Also fetch `/stock/profile2` sector/market-cap → tracked/universe.csv.gz.
    #[arg(long)]
    include_industry: bool,
    /// Best-effort snapshot panels (analyst upside / consensus / fcf_yield /
    /// pe_industry_pctile) from recommendation, price-target, and metric endpoints.
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
pub(crate) struct FinnhubSymbolsArgs {
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    out: PathBuf,
    /// Finnhub exchange/country code to list (default: US).
    #[arg(long, default_value = "US")]
    exchange: String,
    /// Security type filter (default: `Common Stock`). Pass `all` for none.
    #[arg(long, default_value = "Common Stock")]
    security_type: String,
    /// Cap the number of symbols returned.
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, default_value_t = 0)]
    rate_limit: u32,
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
}

pub(crate) fn sync(args: FinnhubSyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let FinnhubSyncArgs {
        api_key,
        out,
        symbols,
        symbols_file,
        index,
        exchange,
        from,
        to,
        include_fundamentals,
        include_industry,
        include_snapshot_factors,
        rate_limit,
        max_retries,
        append,
        resume,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("FINNHUB_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set FINNHUB_API_KEY")?;
    let mut explicit: Vec<String> = symbols
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(path) = symbols_file {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        explicit.extend(yuzu_cli::finnhub::parse_symbols_list(&text));
    }
    let mut seen = std::collections::HashSet::new();
    explicit.retain(|s| seen.insert(s.clone()));
    let mode = if resume {
        yuzu_cli::finnhub::WriteMode::Resume
    } else if append {
        yuzu_cli::finnhub::WriteMode::Append
    } else {
        yuzu_cli::finnhub::WriteMode::Overwrite
    };
    let cfg = yuzu_cli::finnhub::SyncConfig {
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
    let client = yuzu_cli::finnhub::UreqClient::new();

    let membership = if let Some(idx) = index {
        eprintln!("fetching Finnhub index membership…");
        let m = yuzu_cli::finnhub::IndexMembership::fetch(&client, &api_key, idx.into(), &cfg)?;
        let before = explicit.len();
        for s in m.ever_members(from, to) {
            if seen.insert(s.clone()) {
                explicit.push(s);
            }
        }
        eprintln!(
            "index {}: {} ever-members over [{from},{to}] (+{} new)",
            m.series_name(),
            explicit.len(),
            explicit.len() - before
        );
        Some(m)
    } else {
        None
    };
    if explicit.is_empty() {
        return Err("provide --symbols and/or --symbols-file, or --index".into());
    }

    let out_store = pomelo_s3::OutStore::parse(&out)?;
    if membership.is_some() && out_store.is_s3() {
        return Err(
            "--index requires a local --out (membership panel write needs a local trading calendar)"
                .into(),
        );
    }
    let summary = yuzu_cli::finnhub::sync_into(&client, &api_key, &explicit, &out_store, &cfg)?;
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
    if let Some(m) = &membership {
        let (days, cols) =
            yuzu_cli::finnhub::write_index_membership(std::path::Path::new(&out), m, from, to)?;
        eprintln!(
            "wrote panels/{}.csv.gz: {days} days × {cols} symbols (mask with mask(signal, {}))",
            m.series_name(),
            m.series_name()
        );
    }
    Ok(())
}

pub(crate) fn symbols(args: FinnhubSymbolsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let FinnhubSymbolsArgs {
        api_key,
        out,
        exchange,
        security_type,
        limit,
        rate_limit,
        max_retries,
    } = args;
    let api_key = api_key
        .or_else(|| std::env::var("FINNHUB_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-key or set FINNHUB_API_KEY")?;
    let cfg = yuzu_cli::finnhub::SyncConfig {
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    let filter = yuzu_cli::finnhub::SymbolFilter {
        security_type,
        limit,
    };
    let client = yuzu_cli::finnhub::UreqClient::new();
    eprintln!(
        "building symbol universe from Finnhub /stock/symbol ({exchange})… \
         (not a market-cap screener; for index PIT use finnhub-sync --index)"
    );
    let syms = yuzu_cli::finnhub::build_symbol_list(&client, &api_key, &exchange, &cfg, &filter)?;
    if syms.is_empty() {
        return Err("listing returned no symbols (loosen --security-type?)".into());
    }
    let mut body = String::from(
        "# symbols built by `yuzu-cli finnhub-symbols` (/stock/symbol listing)\n\
         # for point-in-time SPX membership use `finnhub-sync --index sp500`\n",
    );
    for s in &syms {
        body.push_str(s);
        body.push('\n');
    }
    std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
    eprintln!("wrote {} symbols to {}", syms.len(), out.display());
    Ok(())
}
