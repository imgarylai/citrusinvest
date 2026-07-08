//! Serializable backtest report — the JSON contract the web app renders
//! (equity/drawdown series + trade list + metrics). Engine computes; frontend draws.

use crate::backtest::{BacktestRun, Trade};
use crate::metrics;
use serde::Serialize;

#[derive(Serialize)]
pub struct Metrics {
    pub total_return: f64,
    pub cagr: f64,
    pub ann_volatility: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub calmar: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub expectancy: f64,
    pub avg_holding_period: f64,
    pub num_trades: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub payoff_ratio: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
    pub max_consecutive_losses: f64,
    pub recovery_factor: f64,
    pub max_drawdown_duration: f64,
    pub time_in_market: f64,
    pub avg_exposure: f64,
    // Benchmark-relative metrics — present only when a benchmark was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benchmark_return: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excess_return: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_error: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_ratio: Option<f64>,
}

#[derive(Serialize)]
pub struct Report {
    pub dates: Vec<i32>,
    pub equity: Vec<f64>,
    pub drawdown: Vec<f64>,
    /// Benchmark equity curve rebased to 1.0 at its first observation, aligned
    /// to `dates`; NaN before the benchmark's first data point. Present only
    /// when a benchmark was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benchmark: Option<Vec<f64>>,
    pub trades: Vec<Trade>,
    pub metrics: Metrics,
}

pub fn build_report(run: BacktestRun) -> Report {
    build_report_with_benchmark(run, None)
}

/// Like [`build_report`], with an optional benchmark equity curve — same
/// length as `run.dates`, rebased to 1.0 (see [`benchmark_equity`]).
pub fn build_report_with_benchmark(run: BacktestRun, benchmark: Option<Vec<f64>>) -> Report {
    let eq = &run.equity;
    let dates = &run.dates;
    let bench = benchmark.as_deref();
    let metrics = Metrics {
        total_return: metrics::total_return(eq),
        cagr: metrics::cagr(eq, dates),
        ann_volatility: metrics::ann_volatility(eq),
        sharpe: metrics::sharpe(eq),
        sortino: metrics::sortino(eq),
        max_drawdown: metrics::max_drawdown(eq),
        calmar: metrics::calmar(eq, dates),
        win_rate: metrics::win_rate(&run.trades),
        profit_factor: metrics::profit_factor(&run.trades),
        expectancy: metrics::expectancy(&run.trades),
        avg_holding_period: metrics::avg_holding_period(&run.trades),
        num_trades: metrics::num_trades(&run.trades),
        avg_win: metrics::avg_win(&run.trades),
        avg_loss: metrics::avg_loss(&run.trades),
        payoff_ratio: metrics::payoff_ratio(&run.trades),
        best_trade: metrics::best_trade(&run.trades),
        worst_trade: metrics::worst_trade(&run.trades),
        max_consecutive_losses: metrics::max_consecutive_losses(&run.trades),
        recovery_factor: metrics::recovery_factor(eq),
        max_drawdown_duration: metrics::max_drawdown_duration(eq),
        time_in_market: metrics::time_in_market(&run.exposure),
        avg_exposure: metrics::avg_exposure(&run.exposure),
        benchmark_return: bench.map(metrics::benchmark_return),
        alpha: bench.map(|b| metrics::alpha(eq, b)),
        beta: bench.map(|b| metrics::beta(eq, b)),
        excess_return: bench.map(|b| metrics::total_return(eq) - metrics::benchmark_return(b)),
        tracking_error: bench.map(|b| metrics::tracking_error(eq, b)),
        information_ratio: bench.map(|b| metrics::information_ratio(eq, b)),
    };
    let drawdown = metrics::drawdown_series(eq);
    Report {
        dates: run.dates,
        equity: run.equity,
        drawdown,
        benchmark,
        trades: run.trades,
        metrics,
    }
}

/// Rebase a benchmark price series onto `dates` as an equity curve: for each
/// date take the last known price at or before it (forward-fill), divided by
/// the first available price; NaN until the benchmark's first observation.
/// `bench_dates`/`bench_px` come from the benchmark panel's first column.
pub fn benchmark_equity(dates: &[i32], bench_dates: &[i32], bench_px: &[f64]) -> Vec<f64> {
    let mut out = vec![f64::NAN; dates.len()];
    let mut i = 0usize; // cursor into bench series
    let mut last = f64::NAN;
    let mut base = f64::NAN;
    for (r, d) in dates.iter().enumerate() {
        while i < bench_dates.len() && bench_dates[i] <= *d {
            if !bench_px[i].is_nan() {
                last = bench_px[i];
                if base.is_nan() && last != 0.0 {
                    base = last;
                }
            }
            i += 1;
        }
        if !last.is_nan() && !base.is_nan() {
            out[r] = last / base;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::{run, BacktestConfig};
    use crate::panel::Panel;

    #[test]
    fn report_bundles_series_and_metrics_and_serializes() {
        let pos = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        )
        .unwrap();
        let report = build_report(run(&pos, &px, None, None, None, &BacktestConfig::default()));
        assert_eq!(report.equity.len(), 3);
        assert_eq!(report.drawdown.len(), 3);
        assert!((report.metrics.total_return - 0.2).abs() < 1e-9);
        // round-trips to JSON
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"sharpe\""));
        assert!(json.contains("\"equity\""));
        // new metrics present in the struct + JSON
        assert_eq!(report.metrics.num_trades, 0.0); // open trade only -> no closed trades
        assert!((report.metrics.avg_exposure - 1.0).abs() < 1e-9); // held every day
        assert!((report.metrics.time_in_market - 1.0).abs() < 1e-9);
        assert!(json.contains("\"recovery_factor\""));
        assert!(json.contains("\"max_drawdown_duration\""));
        assert!(json.contains("\"max_consecutive_losses\""));
        // no benchmark supplied -> benchmark fields absent from the JSON
        assert!(!json.contains("\"benchmark\""));
        assert!(!json.contains("\"alpha\""));
    }

    #[test]
    fn report_with_benchmark_adds_curve_and_relative_metrics() {
        let pos = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        )
        .unwrap();
        let r = run(&pos, &px, None, None, None, &BacktestConfig::default());
        // Benchmark identical to the strategy -> beta 1, excess 0.
        let bench = r.equity.clone();
        let report = build_report_with_benchmark(r, Some(bench));
        let m = &report.metrics;
        assert!((m.beta.unwrap() - 1.0).abs() < 1e-9);
        assert!(m.excess_return.unwrap().abs() < 1e-9);
        assert!(m.tracking_error.unwrap().abs() < 1e-9);
        assert!((m.benchmark_return.unwrap() - 0.2).abs() < 1e-9);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"benchmark\""));
        assert!(json.contains("\"information_ratio\""));
    }

    #[test]
    fn benchmark_equity_aligns_ffills_and_rebases() {
        // Benchmark trades on 3 of the 4 strategy dates; starts one day late.
        let dates = [20240102, 20240103, 20240104, 20240105];
        let bd = [20240103, 20240105];
        let bp = [200.0, 220.0];
        let eq = benchmark_equity(&dates, &bd, &bp);
        assert!(eq[0].is_nan()); // before first benchmark observation
        assert!((eq[1] - 1.0).abs() < 1e-12); // rebased at 200
        assert!((eq[2] - 1.0).abs() < 1e-12); // ffilled through the gap
        assert!((eq[3] - 1.1).abs() < 1e-12); // 220/200
    }
}
