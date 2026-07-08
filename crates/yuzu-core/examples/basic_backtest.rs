//! A minimal, self-contained backtest you can copy, run, and read top to bottom.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p yuzu-core --example basic_backtest
//! ```
//!
//! The pipeline this example walks through is the same one the real app uses:
//!
//! 1. Build a tiny price **panel** (dates × symbols of `f64` prices).
//! 2. Author a strategy in the human-friendly **lemon** DSL and lower it to the
//!    JSON `Expr` spec the engine evaluates.
//! 3. Run the backtest into a [`Report`].
//! 4. Print a few headline metrics.
//!
//! The strategy is deliberately simple: **hold whichever symbol has the highest
//! 2-day average price** — `is_largest(sma(close, 2), 1)` picks the single name
//! whose 2-day simple moving average of close is the largest *value*. The engine
//! turns that boolean position matrix into an equity curve and a trade list, then
//! derives the metrics.

use std::collections::HashMap;

use yuzu_core::backtest::BacktestConfig;
use yuzu_core::panel::Panel;
use yuzu_core::{run_backtest, EvalContext};

fn main() {
    // ---------------------------------------------------------------------
    // 1. The data: a `Panel` is a dense f64 matrix indexed by `dates` (rows,
    //    encoded as YYYYMMDD integers) and `symbols` (columns). Here we make up
    //    six trading days of closing prices for two stocks, ACME and BETA.
    //    `NaN` would mark a missing price; we have none.
    //
    //    Both stocks climb the whole way, but BETA sits at the higher price
    //    level early while ACME rises faster and overtakes it near the end — so
    //    the "hold the highest 2-day average price" rule owns BETA first, then
    //    rotates into ACME. Both legs are held during rises, so the book profits.
    // ---------------------------------------------------------------------
    let dates = vec![20240102, 20240103, 20240104, 20240105, 20240108, 20240109];
    let symbols = vec!["ACME".to_string(), "BETA".to_string()];
    let close = Panel::from_rows(
        dates,
        symbols,
        vec![
            //   ACME   BETA
            vec![20.0, 30.0],
            vec![23.0, 31.0],
            vec![27.0, 32.0],
            vec![32.0, 33.0],
            vec![38.0, 34.0],
            vec![44.0, 35.0],
        ],
    )
    .expect("prices form a valid dates × symbols matrix");

    // The evaluation context maps series names to panels. The strategy below
    // refers to this "close" series by name, and we also mark the backtest off
    // it (the `price_key` argument further down).
    let mut panels = HashMap::new();
    panels.insert("close".to_string(), close);
    let ctx = EvalContext::new(panels);

    // ---------------------------------------------------------------------
    // 2. The strategy. `lemon` is the readable DSL; `lemon::parse` lowers it to
    //    the JSON `Expr` tree the engine deserializes and evaluates.
    //
    //    `is_largest(sma(close, 2), 1)` reads as: of each day's 2-day simple
    //    moving average of close, hold the single (`1`) largest *value* — i.e.
    //    own the one stock trading at the highest recent average price, checked
    //    each day.
    // ---------------------------------------------------------------------
    let strategy_src = "is_largest(sma(close, 2), 1)";
    let spec = lemon::parse(strategy_src).expect("strategy is valid lemon source");
    let spec_json = serde_json::to_string(&spec).expect("spec serializes to JSON");

    // ---------------------------------------------------------------------
    // 3. Run it. `run_backtest` evaluates the spec into a position matrix, then
    //    walks the daily NAV loop over the "close" price series. `BacktestConfig`
    //    defaults to zero fees/taxes and no per-position cap — fine for a demo.
    // ---------------------------------------------------------------------
    let report = run_backtest(&spec_json, &ctx, "close", &BacktestConfig::default())
        .expect("backtest runs to completion");

    // ---------------------------------------------------------------------
    // 4. The results. `Report` bundles the equity/drawdown series, the trade
    //    list, and a `Metrics` struct. Equity is a growth curve based at 1.0, so
    //    a final value of 1.4 means the portfolio grew 40%.
    // ---------------------------------------------------------------------
    let m = &report.metrics;
    println!("Strategy: {strategy_src}");
    println!("Days simulated: {}", report.equity.len());
    println!(
        "Final equity (base 1.0): {:.4}",
        report.equity.last().copied().unwrap_or(f64::NAN)
    );
    println!();
    println!("Headline metrics");
    println!("  total return : {:>8.2}%", m.total_return * 100.0);
    println!("  Sharpe       : {:>8.2}", m.sharpe);
    println!("  max drawdown : {:>8.2}%", m.max_drawdown * 100.0);
    println!("  # trades     : {:>8}", m.num_trades as i64);
    println!("  win rate     : {:>8.2}%", m.win_rate * 100.0);

    // The trade list records each closed (and any still-open) position.
    println!();
    println!("Trades");
    for t in &report.trades {
        let exit = match t.exit_date {
            Some(d) => d.to_string(),
            None => "open".to_string(),
        };
        println!(
            "  {:<4} {} -> {:<6}  return {:>7.2}%  ({} days)",
            t.symbol,
            t.entry_date,
            exit,
            t.ret * 100.0,
            t.period
        );
    }
}
