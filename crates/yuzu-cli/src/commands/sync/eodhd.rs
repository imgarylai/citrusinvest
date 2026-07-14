use std::path::PathBuf;

use clap::Args;

use crate::commands::EodhdIndexArg;

#[derive(Args)]
pub(crate) struct EodhdSyncArgs {
    /// EODHD API token (kept local). Falls back to $EODHD_API_TOKEN or
    /// $EODHD_API_KEY if unset.
    #[arg(long)]
    api_token: Option<String>,
    /// Output data root — local path or `s3://bucket[/prefix]` (same
    /// credential chain as fmp-sync --out).
    #[arg(long)]
    out: String,
    /// Comma-separated tickers (`AAPL` or `AAPL.US`). Or use --symbols-file / --index.
    #[arg(long, value_delimiter = ',')]
    symbols: Vec<String>,
    /// Read symbols from a file (one per line, or comma-separated; `#` comments).
    #[arg(long)]
    symbols_file: Option<PathBuf>,
    /// Sync ever-members of an index over [from,to] and write `panels/in_sp500`
    /// (local --out only). Mutually exclusive with --symbols / --symbols-file.
    #[arg(long, value_enum)]
    index: Option<EodhdIndexArg>,
    /// Default exchange when a bare ticker is given (default: US → `AAPL.US`).
    #[arg(long, default_value = yuzu_cli::eodhd::DEFAULT_EXCHANGE)]
    exchange: String,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 20991231)]
    to: i32,
    /// Also densify annual statement factors → fundamentals/{SYM}.csv.gz
    /// (filing-date visibility; pe/ps/pb/market_cap left NaN historically).
    #[arg(long)]
    include_fundamentals: bool,
    /// Also fetch sector map → tracked/universe.csv.gz from fundamentals General.
    #[arg(long)]
    include_industry: bool,
    /// Also union EODHD delisted names for `--exchange` into the sync list
    /// (survivorship-honest price files). Uses exchange-symbol-list?delisted=1.
    #[arg(long)]
    include_delisted: bool,
    /// Best-effort snapshot panels (analyst_upside_pct, consensus_rating,
    /// fcf_yield, pe_industry_pctile) → panels/. Current-as-of, not history.
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
pub(crate) struct EodhdSymbolsArgs {
    #[arg(long)]
    api_token: Option<String>,
    #[arg(long)]
    out: PathBuf,
    /// Only symbols at/above this market cap in USD (0 = no floor).
    /// Accepts suffixes: 1b, 500m, 10k.
    #[arg(long, default_value = "0", value_parser = yuzu_cli::eodhd::parse_market_cap)]
    min_market_cap: f64,
    /// Exchange filter for the screener (default: us). Pass `all` for none.
    #[arg(long, default_value = "us")]
    exchange: String,
    /// Cap the number of symbols returned.
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, default_value_t = 0)]
    rate_limit: u32,
    #[arg(long, default_value_t = 4)]
    max_retries: u32,
}

pub(crate) fn sync(args: EodhdSyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let EodhdSyncArgs {
        api_token,
        out,
        symbols,
        symbols_file,
        index,
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
    let api_token = api_token
        .or_else(|| std::env::var("EODHD_API_TOKEN").ok())
        .or_else(|| std::env::var("EODHD_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-token or set EODHD_API_TOKEN (or EODHD_API_KEY)")?;
    let had_symbols_flag = !symbols.is_empty();
    let had_file = symbols_file.is_some();
    let mut explicit: Vec<String> = symbols
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(path) = symbols_file {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        explicit.extend(yuzu_cli::eodhd::parse_symbols_list(&text));
    }
    // De-dupe while preserving order.
    let mut seen = std::collections::HashSet::new();
    explicit.retain(|s| seen.insert(s.clone()));

    if (had_symbols_flag || had_file) && index.is_some() {
        return Err("choose one of --symbols/--symbols-file or --index (not both)".into());
    }

    let mode = if resume {
        yuzu_cli::eodhd::WriteMode::Resume
    } else if append {
        yuzu_cli::eodhd::WriteMode::Append
    } else {
        yuzu_cli::eodhd::WriteMode::Overwrite
    };
    let cfg = yuzu_cli::eodhd::SyncConfig {
        from,
        to,
        default_exchange: exchange.clone(),
        include_fundamentals,
        include_industry,
        include_snapshot_factors,
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        mode,
    };
    let client = yuzu_cli::eodhd::UreqClient::new();

    let membership = if let Some(idx) = index {
        eprintln!("fetching EODHD index membership…");
        let m = yuzu_cli::eodhd::IndexMembership::fetch(&client, &api_token, idx.into(), &cfg)?;
        explicit = m.ever_members(from, to);
        seen = explicit.iter().cloned().collect();
        eprintln!(
            "index {}: {} ever-members over [{from},{to}]",
            m.series_name(),
            explicit.len()
        );
        Some(m)
    } else {
        None
    };

    if include_delisted {
        eprintln!("fetching delisted universe from EODHD ({exchange})…");
        let delisted = yuzu_cli::eodhd::fetch_delisted(&client, &api_token, &cfg, &exchange)?;
        let before = explicit.len();
        for d in delisted {
            if seen.insert(d.symbol.clone()) {
                explicit.push(d.symbol);
            }
        }
        eprintln!("delisted: +{} names unioned in", explicit.len() - before);
    }
    if explicit.is_empty() {
        return Err(
            "provide --symbols and/or --symbols-file, or --index, or --include-delisted".into(),
        );
    }
    let out_store = pomelo_s3::OutStore::parse(&out)?;
    if membership.is_some() && out_store.is_s3() {
        return Err(
            "--index requires a local --out (membership panel write needs a local trading calendar)"
                .into(),
        );
    }
    let summary = yuzu_cli::eodhd::sync_into(&client, &api_token, &explicit, &out_store, &cfg)?;
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
            yuzu_cli::eodhd::write_index_membership(std::path::Path::new(&out), m, from, to)?;
        eprintln!(
            "wrote panels/{}.csv.gz: {days} days × {cols} symbols (mask with mask(signal, {}))",
            m.series_name(),
            m.series_name()
        );
    }
    Ok(())
}

pub(crate) fn symbols(args: EodhdSymbolsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let EodhdSymbolsArgs {
        api_token,
        out,
        min_market_cap,
        exchange,
        limit,
        rate_limit,
        max_retries,
    } = args;
    let api_token = api_token
        .or_else(|| std::env::var("EODHD_API_TOKEN").ok())
        .or_else(|| std::env::var("EODHD_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
        .ok_or("provide --api-token or set EODHD_API_TOKEN (or EODHD_API_KEY)")?;
    let cfg = yuzu_cli::eodhd::SyncConfig {
        rate_limit_per_min: rate_limit,
        max_retries,
        backoff_base: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    let filter = yuzu_cli::eodhd::SymbolFilter {
        min_market_cap,
        exchange,
        limit,
    };
    let client = yuzu_cli::eodhd::UreqClient::new();
    eprintln!("building symbol universe from EODHD screener…");
    let syms = yuzu_cli::eodhd::build_symbol_list(&client, &api_token, &cfg, &filter)?;
    if syms.is_empty() {
        return Err("screener returned no symbols (loosen the filters?)".into());
    }
    let mut body = String::from("# symbols built by `yuzu-cli eodhd-symbols`\n");
    for s in &syms {
        body.push_str(s);
        body.push('\n');
    }
    std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
    eprintln!("wrote {} symbols to {}", syms.len(), out.display());
    Ok(())
}
