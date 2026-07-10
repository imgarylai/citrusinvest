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
    // Distribution / tail of daily returns.
    pub best_day: f64,
    pub worst_day: f64,
    pub skew: f64,
    pub kurtosis: f64,
    pub var_95: f64,
    pub cvar_95: f64,
    // Drawdown shape.
    pub avg_drawdown: f64,
    pub ulcer_index: f64,
    // Lookback returns — present only when the backtest is long enough to cover
    // the window (YTD needs a prior calendar-year anchor).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ytd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_year: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_year: Option<f64>,
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
    /// Calendar-bucket return tables (chained off each bucket's closing equity).
    pub monthly_returns: Vec<metrics::PeriodReturn>,
    pub yearly_returns: Vec<metrics::PeriodReturn>,
    /// Rolling 252-day annualized Sharpe / volatility, aligned to `dates`
    /// (NaN → JSON null before the window fills).
    pub rolling_sharpe: Vec<f64>,
    pub rolling_volatility: Vec<f64>,
    /// Bootstrap confidence bands — present only when
    /// `BacktestConfig::bootstrap_samples > 0` (attached by `run_backtest`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<crate::bootstrap::BootstrapSummary>,
    /// Metrics on the post-go-live segment of the equity curve — present only
    /// when `BacktestConfig::live_performance_start` is set (attached by
    /// `run_backtest`). See [`LiveSegment`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<LiveSegment>,
    pub trades: Vec<Trade>,
    pub metrics: Metrics,
}

/// Equity-curve metrics for the segment starting on the first backtest date on
/// or after `BacktestConfig::live_performance_start`.
///
/// Only metrics derivable from the equity curve are reported here; every one of
/// them normalizes by the segment's first equity point, so the block is
/// identical whether or not the segment is rebased to 1.0 (it is, in effect).
/// Trade-based stats are intentionally excluded — a trade can straddle the
/// live boundary, so slicing it by date is ambiguous.
#[derive(Serialize)]
pub struct LiveSegment {
    /// First backtest date (YYYYMMDD) at or after the requested live start.
    pub start: i32,
    /// Number of equity points in the segment (inclusive of `start`).
    pub days: usize,
    pub total_return: f64,
    pub cagr: f64,
    pub ann_volatility: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub calmar: f64,
}

/// Compute the [`LiveSegment`] for the equity curve from the first date on or
/// after `live_start` (YYYYMMDD). Returns `None` when no date qualifies (the
/// live date is past the end of the backtest), matching `skip_serializing_if`.
/// `dates` and `equity` are the report's full-sample series (equal length).
pub fn live_segment(dates: &[i32], equity: &[f64], live_start: i32) -> Option<LiveSegment> {
    let idx = dates.iter().position(|&d| d >= live_start)?;
    let seg_dates = &dates[idx..];
    let seg_eq = &equity[idx..];
    Some(LiveSegment {
        start: seg_dates[0],
        days: seg_eq.len(),
        total_return: metrics::total_return(seg_eq),
        cagr: metrics::cagr(seg_eq, seg_dates),
        ann_volatility: metrics::ann_volatility(seg_eq),
        sharpe: metrics::sharpe(seg_eq),
        sortino: metrics::sortino(seg_eq),
        max_drawdown: metrics::max_drawdown(seg_eq),
        calmar: metrics::calmar(seg_eq, seg_dates),
    })
}

/// Window (trading days) for the rolling series in [`Report`].
pub const ROLLING_WINDOW: usize = 252;

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
        best_day: metrics::best_day(eq),
        worst_day: metrics::worst_day(eq),
        skew: metrics::skew(eq),
        kurtosis: metrics::kurtosis(eq),
        var_95: metrics::var_95(eq),
        cvar_95: metrics::cvar_95(eq),
        avg_drawdown: metrics::avg_drawdown(eq),
        ulcer_index: metrics::ulcer_index(eq),
        ytd: metrics::ytd_return(dates, eq),
        one_year: metrics::trailing_return(dates, eq, 1),
        three_year: metrics::trailing_return(dates, eq, 3),
        benchmark_return: bench.map(metrics::benchmark_return),
        alpha: bench.map(|b| metrics::alpha(eq, b)),
        beta: bench.map(|b| metrics::beta(eq, b)),
        excess_return: bench.map(|b| metrics::total_return(eq) - metrics::benchmark_return(b)),
        tracking_error: bench.map(|b| metrics::tracking_error(eq, b)),
        information_ratio: bench.map(|b| metrics::information_ratio(eq, b)),
    };
    let drawdown = metrics::drawdown_series(eq);
    let monthly_returns = metrics::monthly_returns(dates, eq);
    let yearly_returns = metrics::yearly_returns(dates, eq);
    let rolling_sharpe = metrics::rolling_sharpe(eq, ROLLING_WINDOW);
    let rolling_volatility = metrics::rolling_volatility(eq, ROLLING_WINDOW);
    Report {
        dates: run.dates,
        equity: run.equity,
        drawdown,
        benchmark,
        monthly_returns,
        yearly_returns,
        rolling_sharpe,
        rolling_volatility,
        bootstrap: None,
        live: None,
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
        // distribution / tail + drawdown-shape metrics are always present
        assert!(json.contains("\"best_day\""));
        assert!(json.contains("\"var_95\""));
        assert!(json.contains("\"cvar_95\""));
        assert!(json.contains("\"ulcer_index\""));
        assert!((report.metrics.best_day - 0.1).abs() < 1e-9); // 11/10 - 1
                                                               // lookback returns omitted for a 3-day, single-year backtest
        assert!(report.metrics.ytd.is_none());
        assert!(report.metrics.one_year.is_none());
        assert!(!json.contains("\"ytd\""));
        assert!(!json.contains("\"one_year\""));
        assert!(!json.contains("\"three_year\""));
        // no benchmark supplied -> benchmark fields absent from the JSON
        assert!(!json.contains("\"benchmark\""));
        assert!(!json.contains("\"alpha\""));
        // calendar + rolling series are always present
        assert_eq!(report.monthly_returns.len(), 1); // one month of data
        assert_eq!(report.monthly_returns[0].period, "2024-01");
        assert!((report.monthly_returns[0].ret - 0.2).abs() < 1e-9);
        assert_eq!(report.yearly_returns[0].period, "2024");
        assert_eq!(report.rolling_sharpe.len(), 3); // all NaN (window 252)
        assert!(json.contains("\"monthly_returns\""));
        assert!(json.contains("\"rolling_volatility\""));
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
    fn live_segment_over_full_range_matches_full_sample_metrics() {
        let dates = [20240102, 20240103, 20240104];
        let eq = [100.0, 110.0, 121.0];
        // live start on/before the first date -> whole curve.
        let seg = live_segment(&dates, &eq, 20240101).unwrap();
        assert_eq!(seg.start, 20240102);
        assert_eq!(seg.days, 3);
        assert!((seg.total_return - metrics::total_return(&eq)).abs() < 1e-12);
        assert!((seg.cagr - metrics::cagr(&eq, &dates)).abs() < 1e-12);
        assert!((seg.max_drawdown - metrics::max_drawdown(&eq)).abs() < 1e-12);
        assert!((seg.sharpe - metrics::sharpe(&eq)).abs() < 1e-12);
    }

    #[test]
    fn live_segment_slices_from_first_date_on_or_after_start() {
        let dates = [20240102, 20240103, 20240104];
        let eq = [100.0, 110.0, 121.0];
        // No date equals 20240103-1; first qualifying date is 20240103 itself.
        let seg = live_segment(&dates, &eq, 20240103).unwrap();
        assert_eq!(seg.start, 20240103);
        assert_eq!(seg.days, 2);
        // Segment [110, 121] -> 121/110 - 1 = 0.1, normalized to the segment's
        // own first point (rebasing is a no-op for these metrics).
        assert!((seg.total_return - 0.1).abs() < 1e-12);
        assert!(seg.max_drawdown.abs() < 1e-12); // monotonic up -> no drawdown
    }

    #[test]
    fn live_segment_after_end_is_none() {
        let dates = [20240102, 20240103];
        let eq = [100.0, 110.0];
        assert!(live_segment(&dates, &eq, 20250101).is_none());
    }

    #[test]
    fn report_live_block_present_only_when_set() {
        let dates = [20240102, 20240103, 20240104];
        let eq = [100.0, 110.0, 121.0];
        // Absent by default.
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
        assert!(report.live.is_none());
        assert!(!serde_json::to_string(&report).unwrap().contains("\"live\""));

        // Present once attached (mirrors run_backtest wiring).
        let mut report = report;
        report.live = live_segment(&dates, &eq, 20240103);
        assert!(report.live.is_some());
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"live\""));
        assert!(json.contains("\"total_return\""));
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
