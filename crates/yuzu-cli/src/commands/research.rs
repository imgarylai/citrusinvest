use std::path::PathBuf;

use clap::Args;

use super::{emit, load_grid_variants, CommonArgs, SortArg, Variant};

#[derive(Args)]
pub(crate) struct SweepArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a JSON file: `[{"name": "...", "spec": {...Expr...}}, ...]`.
    #[arg(long)]
    specs: PathBuf,
    /// Metric to rank by.
    #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
    sort: SortArg,
    /// Keep only the top N entries (default: all).
    #[arg(long)]
    top: Option<usize>,
    /// Price series that drives fills and daily returns: open/high/low/close.
    #[arg(long, default_value = "close")]
    price_key: String,
}

#[derive(Args)]
pub(crate) struct GridArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a grid file: `{"spec": {...with "$name"...}, "params": {"name": [values]}}`.
    #[arg(long)]
    grid: PathBuf,
    /// Metric to rank by.
    #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
    sort: SortArg,
    /// Keep only the top N entries (default: all).
    #[arg(long)]
    top: Option<usize>,
    /// Price series that drives fills and daily returns: open/high/low/close.
    #[arg(long, default_value = "close")]
    price_key: String,
}

#[derive(Args)]
pub(crate) struct FactorArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a JSON Expr spec evaluated to the factor panel.
    #[arg(long)]
    spec: PathBuf,
    /// Forward-return horizon in trading days.
    #[arg(long, default_value_t = 1)]
    horizon: usize,
    /// Number of factor quantile buckets.
    #[arg(long, default_value_t = 5)]
    quantiles: usize,
    /// Demean the factor within each sector before ranking (needs an
    /// industry map in the data tree).
    #[arg(long)]
    neutralize_industry: bool,
}

#[derive(Args)]
pub(crate) struct EventArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a JSON Expr spec evaluated to the 0/1 event panel.
    #[arg(long)]
    spec: PathBuf,
    /// Rows before each event to include.
    #[arg(long, default_value_t = 5)]
    pre: usize,
    /// Rows after each event to include.
    #[arg(long, default_value_t = 5)]
    post: usize,
}

#[derive(Args)]
pub(crate) struct LookaheadArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a JSON Expr spec file.
    #[arg(long)]
    spec: PathBuf,
    /// Days to lag the position matrix in the comparison run.
    #[arg(long, default_value_t = 1)]
    shift_days: usize,
    /// Run the full decay profile (shifts 1,2,3,5,10,21) instead of a
    /// single comparison; --shift-days is ignored.
    #[arg(long)]
    profile: bool,
}

#[derive(Args)]
pub(crate) struct WalkforwardArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a grid file (same format as `grid`).
    #[arg(long)]
    grid: PathBuf,
    /// In-sample window length in trading days.
    #[arg(long, default_value_t = 504)]
    train_days: usize,
    /// Out-of-sample window length in trading days.
    #[arg(long, default_value_t = 126)]
    test_days: usize,
    /// Indicator warmup rows carried into each window (default: auto —
    /// the largest window argument found in any variant).
    #[arg(long)]
    warmup_days: Option<usize>,
    /// Metric used to pick the in-sample winner.
    #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
    sort: SortArg,
}

pub(crate) fn sweep(args: SweepArgs) -> Result<(), Box<dyn std::error::Error>> {
    let SweepArgs {
        common,
        specs,
        sort,
        top,
        price_key,
    } = args;
    let cfg = common.config();
    let raw = std::fs::read_to_string(&specs)?;
    let parsed: Vec<Variant> = serde_json::from_str(&raw)?;
    let variants: Vec<(String, String)> = parsed
        .into_iter()
        .map(|v| {
            let spec_str = serde_json::to_string(&v.spec)?;
            Ok((v.name, spec_str))
        })
        .collect::<Result<_, serde_json::Error>>()?;

    let mut board = yuzu_cli::run_sweep(
        &common.data,
        &variants,
        common.from,
        common.to,
        &cfg,
        &price_key,
        sort.into(),
    );
    if let Some(n) = top {
        board.truncate(n);
    }
    emit(&common.out, serde_json::to_string_pretty(&board)?)?;
    Ok(())
}

pub(crate) fn grid(args: GridArgs) -> Result<(), Box<dyn std::error::Error>> {
    let GridArgs {
        common,
        grid,
        sort,
        top,
        price_key,
    } = args;
    let cfg = common.config();
    let variants = load_grid_variants(&grid)?;
    let mut board = yuzu_cli::run_sweep(
        &common.data,
        &variants,
        common.from,
        common.to,
        &cfg,
        &price_key,
        sort.into(),
    );
    if let Some(n) = top {
        board.truncate(n);
    }
    emit(&common.out, serde_json::to_string_pretty(&board)?)?;
    Ok(())
}

pub(crate) fn factor(args: FactorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let FactorArgs {
        common,
        spec,
        horizon,
        quantiles,
        neutralize_industry,
    } = args;
    let spec_json = std::fs::read_to_string(&spec)?;
    let report = yuzu_cli::run_factor(
        &common.data,
        &spec_json,
        common.from,
        common.to,
        horizon,
        quantiles,
        neutralize_industry,
    )?;
    emit(&common.out, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}

pub(crate) fn event(args: EventArgs) -> Result<(), Box<dyn std::error::Error>> {
    let EventArgs {
        common,
        spec,
        pre,
        post,
    } = args;
    let spec_json = std::fs::read_to_string(&spec)?;
    let report = yuzu_cli::run_event(&common.data, &spec_json, common.from, common.to, pre, post)?;
    emit(&common.out, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}

pub(crate) fn lookahead(args: LookaheadArgs) -> Result<(), Box<dyn std::error::Error>> {
    let LookaheadArgs {
        common,
        spec,
        shift_days,
        profile,
    } = args;
    let cfg = common.config();
    let spec_json = std::fs::read_to_string(&spec)?;
    let json = if profile {
        let report = yuzu_cli::run_lookahead_profile(
            &common.data,
            &spec_json,
            common.from,
            common.to,
            yuzu_cli::PROFILE_SHIFTS,
            &cfg,
        )?;
        serde_json::to_string_pretty(&report)?
    } else {
        let report = yuzu_cli::run_lookahead(
            &common.data,
            &spec_json,
            common.from,
            common.to,
            shift_days,
            &cfg,
        )?;
        serde_json::to_string_pretty(&report)?
    };
    emit(&common.out, json)?;
    Ok(())
}

pub(crate) fn walkforward(args: WalkforwardArgs) -> Result<(), Box<dyn std::error::Error>> {
    let WalkforwardArgs {
        common,
        grid,
        train_days,
        test_days,
        warmup_days,
        sort,
    } = args;
    let cfg = common.config();
    let variants = load_grid_variants(&grid)?;
    let report = yuzu_cli::run_walkforward(
        &common.data,
        &variants,
        &yuzu_cli::WalkForwardParams {
            from: common.from,
            to: common.to,
            train_days,
            test_days,
            sort_by: sort.into(),
            warmup_days,
        },
        &cfg,
    )?;
    emit(&common.out, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}
