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
}

#[derive(Serialize)]
pub struct Report {
    pub dates: Vec<i32>,
    pub equity: Vec<f64>,
    pub drawdown: Vec<f64>,
    pub trades: Vec<Trade>,
    pub metrics: Metrics,
}

pub fn build_report(run: BacktestRun) -> Report {
    let eq = &run.equity;
    let dates = &run.dates;
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
    };
    let drawdown = metrics::drawdown_series(eq);
    Report { dates: run.dates, equity: run.equity, drawdown, trades: run.trades, metrics }
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
        let report = build_report(run(&pos, &px, None, None, &BacktestConfig::default()));
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
    }
}
