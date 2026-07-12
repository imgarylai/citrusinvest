// TypeScript mirror of the engine's Report JSON (crates/yuzu-core/src/report.rs).
// The engine serializes f64 NaN as JSON null, so every numeric field that can
// be undefined-by-math is `number | null`; fields the engine omits entirely
// (serde skip_serializing_if) are optional.

/** One calendar bucket: `period` is "2016-03" (monthly) or "2016" (yearly). */
export interface PeriodReturn {
  period: string;
  ret: number | null;
}

/** Percentile band from the circular block bootstrap. */
export interface BootstrapCi {
  p05: number | null;
  p50: number | null;
  p95: number | null;
}

export interface BootstrapSummary {
  n_samples: number;
  block_len: number;
  sharpe: BootstrapCi;
  cagr: BootstrapCi;
  max_drawdown: BootstrapCi;
}

/** Equity-curve metrics on the post-go-live slice (config.live_performance_start). */
export interface LiveSegment {
  start: number;
  days: number;
  total_return: number | null;
  cagr: number | null;
  ann_volatility: number | null;
  sharpe: number | null;
  sortino: number | null;
  max_drawdown: number | null;
  calmar: number | null;
}

export interface Trade {
  symbol: string;
  /** YYYYMMDD */
  entry_date: number;
  /** YYYYMMDD; null while the position is still open (mark-to-market). */
  exit_date: number | null;
  /** Net return of the round trip (fees deducted; MTM for open trades). */
  ret: number;
  /** Trading days held. */
  period: number;
  /** Max adverse excursion (worst interim return), when computable. */
  mae: number | null;
  /** Max favorable excursion (best interim return), when computable. */
  mfe: number | null;
  entry_price: number | null;
  exit_price?: number | null;
  side: 'long' | 'short';
}

export interface Metrics {
  total_return: number | null;
  cagr: number | null;
  ann_volatility: number | null;
  sharpe: number | null;
  sortino: number | null;
  max_drawdown: number | null;
  calmar: number | null;
  win_rate: number | null;
  profit_factor: number | null;
  expectancy: number | null;
  avg_holding_period: number | null;
  num_trades: number | null;
  avg_win: number | null;
  avg_loss: number | null;
  payoff_ratio: number | null;
  best_trade: number | null;
  worst_trade: number | null;
  max_consecutive_losses: number | null;
  recovery_factor: number | null;
  max_drawdown_duration: number | null;
  time_in_market: number | null;
  avg_exposure: number | null;
  best_day: number | null;
  worst_day: number | null;
  skew: number | null;
  kurtosis: number | null;
  var_95: number | null;
  cvar_95: number | null;
  avg_drawdown: number | null;
  ulcer_index: number | null;
  // Lookback returns — omitted when the backtest is too short.
  ytd?: number | null;
  one_year?: number | null;
  three_year?: number | null;
  // Benchmark-relative — present only when a benchmark was supplied.
  benchmark_return?: number | null;
  alpha?: number | null;
  beta?: number | null;
  excess_return?: number | null;
  tracking_error?: number | null;
  information_ratio?: number | null;
}

export interface Report {
  /** YYYYMMDD, one per trading day. */
  dates: number[];
  /** NAV rebased to 1.0 at the start. */
  equity: number[];
  /** Fraction below the running peak (≤ 0). */
  drawdown: number[];
  /** Benchmark curve rebased to 1.0, aligned to `dates`; null before its first observation. */
  benchmark?: (number | null)[];
  monthly_returns: PeriodReturn[];
  yearly_returns: PeriodReturn[];
  /** Rolling 252-day annualized Sharpe / volatility; null before the window fills. */
  rolling_sharpe: (number | null)[];
  rolling_volatility: (number | null)[];
  bootstrap?: BootstrapSummary;
  live?: LiveSegment;
  trades: Trade[];
  metrics: Metrics;
}
