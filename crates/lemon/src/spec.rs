//! [`Expr`]: the serializable strategy AST. A JSON tree (tagged by `"op"`) that the
//! website and the batch runner both emit; [`crate::eval`] walks it against a data
//! context to produce a position matrix.

use serde::Deserialize;

fn default_true() -> bool {
    true
}

fn default_stoch_d() -> usize {
    3
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op")]
pub enum Expr {
    /// A raw input series by name (e.g. close, pe, revenue_growth).
    Data { name: String },
    /// A constant scalar value, broadcast across the panel.
    Const { value: f64 },
    /// Simple moving average of `of` over `n` days.
    Average { of: Box<Expr>, n: usize },
    /// Exponential moving average of `of` over `n` days.
    Ema { of: Box<Expr>, n: usize },
    /// Rolling standard deviation of `of` over `n` days.
    Std { of: Box<Expr>, n: usize },
    /// Relative Strength Index of `of` over `n` days.
    Rsi { of: Box<Expr>, n: usize },
    /// Percentage change of `of` over `n` days.
    PctChange { of: Box<Expr>, n: usize },
    /// 1 where `of` rose for `n` consecutive days, else 0.
    Rise { of: Box<Expr>, n: usize },
    /// `of` lagged forward by `n` days.
    Shift { of: Box<Expr>, n: usize },
    /// Rolling maximum of `of` over `n` days.
    RollingMax { of: Box<Expr>, n: usize },
    /// Average True Range over `n` days from high/low/close.
    Atr {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Normalized ATR (percent) over `n` days.
    Natr {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Williams %R over `n` days.
    WillR {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Commodity Channel Index over `n` days.
    Cci {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Stochastic %K over `n` days.
    StochK {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Stochastic %D: `d`-day average of %K over `n` days.
    StochD {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
        #[serde(default = "default_stoch_d")]
        d: usize,
    },
    /// Aroon Up over `n` days from high.
    AroonUp { high: Box<Expr>, n: usize },
    /// Aroon Down over `n` days from low.
    AroonDown { low: Box<Expr>, n: usize },
    /// Average Directional Index over `n` days.
    Adx {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Plus Directional Indicator (+DI) over `n` days.
    PlusDi {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// Minus Directional Indicator (−DI) over `n` days.
    MinusDi {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        n: usize,
    },
    /// On-Balance Volume from close and volume.
    Obv { close: Box<Expr>, volume: Box<Expr> },
    /// Money Flow Index over `n` days.
    Mfi {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        volume: Box<Expr>,
        n: usize,
    },
    /// Volume-Weighted Average Price over `n` days from high/low/close/volume.
    Vwap {
        high: Box<Expr>,
        low: Box<Expr>,
        close: Box<Expr>,
        volume: Box<Expr>,
        n: usize,
    },
    /// 1 where `of` fell for `n` consecutive days, else 0.
    Fall { of: Box<Expr>, n: usize },
    /// 1 for the `n` highest values per row (cross-section), else 0.
    IsLargest { of: Box<Expr>, n: usize },
    /// 1 for the `n` lowest values per row (cross-section), else 0.
    IsSmallest { of: Box<Expr>, n: usize },
    /// 1 where `of` held true at least `nsatisfy` times within the last `nwindow` rows.
    Sustain {
        of: Box<Expr>,
        nwindow: usize,
        nsatisfy: Option<usize>,
    },
    /// 1 on the row where `of` turns false→true (rising edge).
    IsEntry { of: Box<Expr> },
    /// 1 on the row where `of` turns true→false (falling edge).
    IsExit { of: Box<Expr> },
    /// 1 where `l` is greater than `r`, else 0.
    Gt { l: Box<Expr>, r: Box<Expr> },
    /// 1 where `l` is less than `r`, else 0.
    Lt { l: Box<Expr>, r: Box<Expr> },
    /// 1 where `l` is greater than or equal to `r`, else 0.
    Ge { l: Box<Expr>, r: Box<Expr> },
    /// 1 where `l` is less than or equal to `r`, else 0.
    Le { l: Box<Expr>, r: Box<Expr> },
    /// Logical AND of two boolean panels.
    And { l: Box<Expr>, r: Box<Expr> },
    /// Logical OR of two boolean panels.
    Or { l: Box<Expr>, r: Box<Expr> },
    /// Element-wise `l` + `r`.
    Add { l: Box<Expr>, r: Box<Expr> },
    /// Element-wise `l` − `r`.
    Sub { l: Box<Expr>, r: Box<Expr> },
    /// Element-wise `l` × `r`.
    Mul { l: Box<Expr>, r: Box<Expr> },
    /// Element-wise `l` ÷ `r`.
    Div { l: Box<Expr>, r: Box<Expr> },
    /// Negation (−`of`).
    Neg { of: Box<Expr> },
    /// Ceiling of `of`.
    Ceil { of: Box<Expr> },
    /// Cross-sectional rank of `of` per row; `pct` for 0..1 percentile, `ascending` sets direction.
    Rank {
        of: Box<Expr>,
        #[serde(default = "default_true")]
        pct: bool,
        #[serde(default = "default_true")]
        ascending: bool,
    },
    /// `of` kept only where `by` is true; elsewhere dropped.
    Mask { of: Box<Expr>, by: Box<Expr> },
    /// Stateful rotation: enter on `entry`, exit on `exit`, hold up to `nstocks_limit` (prioritised by `rank`), with optional stop_loss/take_profit/trail_stop/trail_stop_activation.
    HoldUntil {
        entry: Box<Expr>,
        exit: Box<Expr>,
        nstocks_limit: Option<usize>,
        rank: Option<Box<Expr>>,
        #[serde(default)]
        stop_loss: Option<f64>,
        #[serde(default)]
        take_profit: Option<f64>,
        #[serde(default)]
        trail_stop: Option<f64>,
        #[serde(default)]
        trail_stop_activation: Option<f64>,
    },
    /// Hold `of`, refreshing on calendar `freq` (W/ME/QE) or on dates where `on` is true.
    Rebalance {
        of: Box<Expr>,
        #[serde(default)]
        freq: Option<String>,
        #[serde(default)]
        on: Option<Box<Expr>>,
    },
    /// Cross-sectionally regress `of` against the `by` factors, optionally adding a constant.
    Neutralize {
        of: Box<Expr>,
        by: Vec<Expr>,
        #[serde(default = "default_true")]
        add_const: bool,
    },
    /// Neutralize `of` within each industry/sector.
    NeutralizeIndustry {
        of: Box<Expr>,
        #[serde(default = "default_true")]
        add_const: bool,
    },
    /// Rank `of` within each industry, optionally limited to `categories`.
    IndustryRank {
        of: Box<Expr>,
        #[serde(default)]
        categories: Option<Vec<String>>,
    },
    /// Aggregate `of` within each industry using `agg` (e.g. mean).
    GroupbyCategory { of: Box<Expr>, agg: String },
}
