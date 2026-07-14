use std::path::PathBuf;

use clap::Args;

use super::{emit, CommonArgs};

#[derive(Args)]
pub(crate) struct RunArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Path to a JSON Expr spec file.
    #[arg(long)]
    spec: PathBuf,
    /// Price series that drives fills and daily returns: open/high/low/close.
    /// For next-open execution, keep close-based signals and lag them
    /// (`shift(signal, 1)`), then set `--price-key open`.
    #[arg(long, default_value = "close")]
    price_key: String,
    /// Restrict the universe to these symbols (comma-separated, e.g.
    /// AAPL,MSFT). Cross-sectional ops then see exactly this universe.
    /// Every listed symbol must exist in the data tree. Beware: a list
    /// frozen today implies survivorship bias in a historical run.
    #[arg(long, value_delimiter = ',')]
    symbols: Option<Vec<String>>,
}

pub(crate) fn run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    let RunArgs {
        common,
        spec,
        price_key,
        symbols,
    } = args;
    let cfg = common.config();
    let spec_json = std::fs::read_to_string(&spec)?;
    let report = yuzu_cli::run_single(
        &common.data,
        &spec_json,
        common.from,
        common.to,
        &cfg,
        &price_key,
        symbols.as_deref(),
    )?;
    emit(&common.out, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}
