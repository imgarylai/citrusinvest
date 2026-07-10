//! Backtest and execution-stop configuration.

/// How a triggered stop fills. `Touched` (the realistic default) fills at the
/// stop level when the bar's range straddled it, or at the day's **open** when
/// the bar gapped through it (a worse-than-stop fill you couldn't avoid).
/// `Close` fills at the day's close — the "end-of-day rule" execution style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopFill {
    #[default]
    Touched,
    Close,
}

/// Execution-layer stops applied by the NAV loop to whatever position book it is
/// given (not just `hold_until`). Each holding is tracked from its entry price;
/// when a day's prices cross a level the position is force-exited to cash at the
/// [`StopFill`] price and re-entry into that name is blocked until the position
/// signal drops and re-adds it. All-off by default, so an unset `StopConfig`
/// leaves the equity curve (and every golden) unchanged.
#[derive(Debug, Clone, Copy)]
pub struct StopConfig {
    /// Exit when the return from entry falls to `−stop_loss` (e.g. `0.08` = −8%).
    /// `f64::NEG_INFINITY` (the default) disables it.
    pub stop_loss: f64,
    /// Exit when the return from entry rises to `+take_profit`.
    /// `f64::INFINITY` (the default) disables it.
    pub take_profit: f64,
    /// Exit when the return drops `trail_stop` below the best return seen since
    /// entry (a trailing stop). `f64::INFINITY` (the default) disables it.
    pub trail_stop: f64,
    /// The trailing stop only arms once the return since entry first reaches
    /// `+trail_stop_activation`. `0.0` arms immediately.
    pub trail_stop_activation: f64,
    /// How a triggered stop fills (default [`StopFill::Touched`]).
    pub fill: StopFill,
}

impl Default for StopConfig {
    fn default() -> Self {
        StopConfig {
            stop_loss: f64::NEG_INFINITY,
            take_profit: f64::INFINITY,
            trail_stop: f64::INFINITY,
            trail_stop_activation: 0.0,
            fill: StopFill::Touched,
        }
    }
}

impl StopConfig {
    /// True when no stop level is set — the NAV loop skips the stop pass entirely.
    pub(crate) fn is_off(&self) -> bool {
        self.stop_loss == f64::NEG_INFINITY
            && self.take_profit == f64::INFINITY
            && self.trail_stop == f64::INFINITY
    }

    /// True when at least one stop level is set. Callers use this to decide
    /// whether to load the OHLC panels the `Touched` fill needs.
    pub fn is_active(&self) -> bool {
        !self.is_off()
    }

    /// Build from optional levels (`None` = that stop off) — the shape the CLI,
    /// server, and WASM request configs use, so the `±INF` sentinels live in one
    /// place.
    pub fn from_options(
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        trail_stop: Option<f64>,
        trail_stop_activation: f64,
        fill: StopFill,
    ) -> Self {
        StopConfig {
            stop_loss: stop_loss.unwrap_or(f64::NEG_INFINITY),
            take_profit: take_profit.unwrap_or(f64::INFINITY),
            trail_stop: trail_stop.unwrap_or(f64::INFINITY),
            trail_stop_activation,
            fill,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BacktestConfig {
    pub fee_ratio: f64,
    pub tax_ratio: f64,
    pub position_limit: f64,
    /// Slippage charged per unit of turnover (both sides), on top of `fee_ratio`.
    /// A crude stand-in for market impact / spread: `0.0005` = 5 bps per trade
    /// leg. `0.0` (the default) disables it.
    pub slippage_ratio: f64,
    /// Notional book size in dollars, used only by the liquidity cap below to
    /// convert weights into dollar positions. `0.0` (the default) disables the cap.
    pub initial_capital: f64,
    /// Max fraction of a symbol's daily dollar volume the book may hold:
    /// `|w| <= max_participation * price * volume / initial_capital`. Requires a
    /// volume panel and `initial_capital > 0`. `0.0` (the default) disables it.
    /// The cap is measured against `initial_capital`, not compounded equity.
    pub max_participation: f64,
    /// Square-root market-impact coefficient. On each rebalance, every traded
    /// cell pays `impact_coef * sqrt(participation)` per unit of turnover,
    /// where `participation = |Δw| * initial_capital / dollar_volume`, capped
    /// at 1. Requires `initial_capital > 0` and a volume panel; a cell with
    /// missing or zero dollar volume pays only the flat `slippage_ratio`.
    /// `0.0` (the default) disables it and reproduces the flat-cost path
    /// exactly.
    pub impact_coef: f64,
    /// After this many consecutive missing-price rows a symbol is treated as
    /// delisted: the position is force-closed at its last valid price (less
    /// `delist_haircut`) and re-entry is blocked until prices resume. `0` (the
    /// default) keeps the legacy behavior (a dead position freezes at its last
    /// value — survivorship-friendly, beware).
    pub delist_after: usize,
    /// Fraction of a force-closed position's value written off on delisting:
    /// `0.0` = exit at the last valid price, `1.0` = total loss. Shorts gain
    /// symmetrically. Only used when `delist_after > 0`.
    pub delist_haircut: f64,
    /// Name of a series in the `EvalContext` to compare against (e.g. a panel
    /// holding SPY closes). When set, `run_backtest` adds a rebased benchmark
    /// curve and benchmark-relative metrics (alpha/beta/excess/tracking
    /// error/information ratio) to the report. The NAV loop ignores it.
    pub benchmark_key: Option<String>,
    /// Number of circular-block-bootstrap resamples of the daily returns; the
    /// report gains p05/p50/p95 bands for Sharpe/CAGR/max drawdown. `0` (the
    /// default) disables it. Deterministic (fixed internal seed).
    pub bootstrap_samples: usize,
    /// Bootstrap block length in trading days; `0` (the default) auto-selects
    /// `⌊√n⌋`. Only used when `bootstrap_samples > 0`.
    pub bootstrap_block: usize,
    /// Date (YYYYMMDD) a strategy went live. When set, `run_backtest` adds a
    /// `live` block to the report with equity-curve metrics computed on the
    /// segment from the first backtest date on or after this day. `None` (the
    /// default) omits the block. The NAV loop ignores it — it is a report-only
    /// concern and does not change the full-sample equity curve.
    pub live_performance_start: Option<i32>,
    /// Execution-layer stops (stop-loss / take-profit / trailing). All-off by
    /// default; see [`StopConfig`]. Requires the OHLC panels for `Touched` fills.
    pub stops: StopConfig,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        BacktestConfig {
            fee_ratio: 0.0,
            tax_ratio: 0.0,
            position_limit: 0.0,
            slippage_ratio: 0.0,
            initial_capital: 0.0,
            max_participation: 0.0,
            impact_coef: 0.0,
            delist_after: 0,
            delist_haircut: 0.0,
            benchmark_key: None,
            bootstrap_samples: 0,
            bootstrap_block: 0,
            live_performance_start: None,
            stops: StopConfig::default(),
        }
    }
}
