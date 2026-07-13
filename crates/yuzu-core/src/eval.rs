//! Evaluates an [`Expr`] tree against a data context (`HashMap<String, Panel>`).
//! [`run_strategy`] parses a JSON spec and evaluates it to a position matrix.
//!
//! Internally, evaluation borrows `Data` leaves from the context so multi-reference
//! expression trees do not deep-copy dense series at every leaf. The public API
//! still returns an owned [`Panel`].

use crate::error::EngineError;
use crate::ops::rebalance::Freq;
use crate::ops::rotation::HoldUntilOpts;
use crate::ops::ta;
use crate::panel::{bool_to_f64, Panel};
use crate::spec::Expr;
use std::collections::HashMap;

/// Data context for evaluation: numeric `panels` keyed by series name, plus a
/// `symbol -> sector` classification used by the neutralization/grouping ops.
/// `industry` is empty for strategies that use no industry ops.
pub struct EvalContext {
    pub panels: HashMap<String, Panel>,
    pub industry: HashMap<String, String>,
}

impl EvalContext {
    pub fn new(panels: HashMap<String, Panel>) -> Self {
        EvalContext {
            panels,
            industry: HashMap::new(),
        }
    }
}

impl From<HashMap<String, Panel>> for EvalContext {
    fn from(panels: HashMap<String, Panel>) -> Self {
        EvalContext::new(panels)
    }
}

pub fn run_strategy(spec_json: &str, ctx: &EvalContext) -> Result<Panel, EngineError> {
    let expr: Expr =
        serde_json::from_str(spec_json).map_err(|e| EngineError::SpecParse(e.to_string()))?;
    eval(&expr, ctx)
}

/// A `Const` operand carries a scalar; anything else evaluates to a panel.
fn as_const(e: &Expr) -> Option<f64> {
    match e {
        Expr::Const { value } => Some(*value),
        _ => None,
    }
}

/// Intermediate eval result: `Data` leaves borrow from the context; every
/// computed node owns a fresh panel. This avoids deep-copying dense series
/// matrices at every data-series reference in the expression tree.
enum EvalOut<'a> {
    Borrowed(&'a Panel),
    Owned(Panel),
}

impl<'a> EvalOut<'a> {
    fn as_panel(&self) -> &Panel {
        match self {
            EvalOut::Borrowed(p) => p,
            EvalOut::Owned(p) => p,
        }
    }

    fn into_owned(self) -> Panel {
        match self {
            EvalOut::Borrowed(p) => {
                #[cfg(test)]
                borrowed_into_owned_inc();
                p.clone()
            }
            EvalOut::Owned(p) => p,
        }
    }
}

// Test-only counters: data-leaf visits vs deep clones forced by into_owned.
// After this change, multi-use intermediate `Data` leaves should visit many
// times with zero borrowed→owned clones when the root is a computed panel.
#[cfg(test)]
thread_local! {
    static BORROWED_INTO_OWNED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static DATA_LEAF_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn borrowed_into_owned_inc() {
    BORROWED_INTO_OWNED.with(|c| c.set(c.get() + 1));
}

#[cfg(test)]
fn data_leaf_hit_inc() {
    DATA_LEAF_HITS.with(|c| c.set(c.get() + 1));
}

#[cfg(test)]
fn take_eval_counters() -> (u64 /* data hits */, u64 /* borrowed clones */) {
    let hits = DATA_LEAF_HITS.with(|c| c.replace(0));
    let clones = BORROWED_INTO_OWNED.with(|c| c.replace(0));
    (hits, clones)
}

/// Evaluate a binary numeric/comparison op, broadcasting a `Const` on either
/// side against the other operand's panel (`panel <op> scalar`). Rejects
/// `const <op> const` — there's no panel shape to broadcast onto.
fn num_binop<'a>(
    l: &Expr,
    r: &Expr,
    ctx: &'a EvalContext,
    f: impl Fn(f64, f64) -> f64,
) -> Result<EvalOut<'a>, EngineError> {
    match (as_const(l), as_const(r)) {
        (Some(_), Some(_)) => Err(EngineError::BothOperandsConst),
        (Some(a), None) => Ok(EvalOut::Owned(
            eval_out(r, ctx)?.as_panel().scalar_lhs(a, f),
        )),
        (None, Some(b)) => Ok(EvalOut::Owned(
            eval_out(l, ctx)?.as_panel().scalar_rhs(b, f),
        )),
        (None, None) => {
            let left = eval_out(l, ctx)?;
            let right = eval_out(r, ctx)?;
            Ok(EvalOut::Owned(left.as_panel().ewise(right.as_panel(), f)))
        }
    }
}

/// Evaluate an expression tree. The top-level match is grouped by op family and
/// stays **exhaustive** — a new `Expr` variant that is not listed here fails to
/// compile. Family helpers also match exhaustively over their subset.
pub fn eval(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    Ok(eval_out(expr, ctx)?.into_owned())
}

fn eval_out<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    match expr {
        // Leaves
        e @ (Data { .. } | Const { .. }) => eval_leaf(e, ctx),

        // Unary time-series indicators
        e @ (Average { .. }
        | Ema { .. }
        | Std { .. }
        | Rsi { .. }
        | PctChange { .. }
        | Rise { .. }
        | Fall { .. }
        | Shift { .. }
        | RollingMax { .. }
        | RollingMin { .. }
        | BollingerMid { .. }
        | BollingerUpper { .. }
        | BollingerLower { .. }
        | Macd { .. }
        | MacdSignal { .. }
        | MacdHist { .. }
        | DonchianHigh { .. }
        | DonchianLow { .. }
        | DonchianMid { .. }) => eval_ts(e, ctx),

        // Multi-series technical analysis (high/low/close/volume)
        e @ (Atr { .. }
        | Natr { .. }
        | WillR { .. }
        | Cci { .. }
        | StochK { .. }
        | StochD { .. }
        | AroonUp { .. }
        | AroonDown { .. }
        | Adx { .. }
        | PlusDi { .. }
        | MinusDi { .. }
        | Obv { .. }
        | Mfi { .. }
        | Vwap { .. }) => eval_ta(e, ctx),

        // Cross-section selection, ranks, preprocess, entry/exit signals
        e @ (IsLargest { .. }
        | IsSmallest { .. }
        | Sustain { .. }
        | IsEntry { .. }
        | IsExit { .. }
        | ExitWhen { .. }
        | QuantileRow { .. }
        | Winsorize { .. }
        | Zscore { .. }
        | Bucket { .. }
        | Demean { .. }
        | Rank { .. }) => eval_cs(e, ctx),

        // Arithmetic, comparisons, logic, mask
        e @ (Gt { .. }
        | Lt { .. }
        | Ge { .. }
        | Le { .. }
        | And { .. }
        | Or { .. }
        | Not { .. }
        | Add { .. }
        | Sub { .. }
        | Mul { .. }
        | Div { .. }
        | Neg { .. }
        | Ceil { .. }
        | Mask { .. }) => eval_arith(e, ctx),

        // Portfolio construction / scheduling
        e @ (NormalizeRow { .. } | VolTarget { .. } | HoldUntil { .. } | Rebalance { .. }) => {
            eval_portfolio(e, ctx)
        }

        // Industry / neutralization
        e @ (Neutralize { .. }
        | NeutralizeIndustry { .. }
        | IndustryRank { .. }
        | CapIndustry { .. }
        | GroupbyCategory { .. }
        | InSector { .. }) => eval_industry(e, ctx),
    }
}

fn eval_leaf<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    match expr {
        Data { name } => {
            #[cfg(test)]
            data_leaf_hit_inc();
            let p = ctx
                .panels
                .get(name)
                .ok_or_else(|| EngineError::UnknownSeries {
                    name: name.clone(),
                })?;
            Ok(EvalOut::Borrowed(p))
        }
        Const { value } => {
            // Const is only meaningful inside scalar ops, handled by callers.
            Err(EngineError::BareConst { value: *value })
        }
        _ => unreachable!("eval_leaf: not a leaf"),
    }
}

fn eval_ts<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    Ok(EvalOut::Owned(match expr {
        Average { of, n } => eval_out(of, ctx)?.as_panel().average(*n),
        Ema { of, n } => eval_out(of, ctx)?.as_panel().ema(*n),
        Std { of, n } => eval_out(of, ctx)?.as_panel().rolling_std(*n),
        Rsi { of, n } => eval_out(of, ctx)?.as_panel().rsi(*n),
        PctChange { of, n } => eval_out(of, ctx)?.as_panel().pct_change(*n),
        Rise { of, n } => eval_out(of, ctx)?.as_panel().rise(*n),
        Fall { of, n } => eval_out(of, ctx)?.as_panel().fall(*n),
        Shift { of, n } => eval_out(of, ctx)?.as_panel().shift(*n),
        RollingMax { of, n } => eval_out(of, ctx)?.as_panel().rolling_max(*n),
        RollingMin { of, n } => eval_out(of, ctx)?.as_panel().rolling_min(*n),
        BollingerMid { of, n } => eval_out(of, ctx)?.as_panel().bollinger_mid(*n),
        BollingerUpper { of, n, k } => eval_out(of, ctx)?.as_panel().bollinger_upper(*n, *k),
        BollingerLower { of, n, k } => eval_out(of, ctx)?.as_panel().bollinger_lower(*n, *k),
        Macd { of, fast, slow } => eval_out(of, ctx)?.as_panel().macd(*fast, *slow),
        MacdSignal {
            of,
            fast,
            slow,
            signal,
        } => eval_out(of, ctx)?
            .as_panel()
            .macd_signal(*fast, *slow, *signal),
        MacdHist {
            of,
            fast,
            slow,
            signal,
        } => eval_out(of, ctx)?
            .as_panel()
            .macd_hist(*fast, *slow, *signal),
        DonchianHigh { of, n } => eval_out(of, ctx)?.as_panel().donchian_high(*n),
        DonchianLow { of, n } => eval_out(of, ctx)?.as_panel().donchian_low(*n),
        DonchianMid { of, n } => eval_out(of, ctx)?.as_panel().donchian_mid(*n),
        _ => unreachable!("eval_ts: not a unary TS op"),
    }))
}

fn eval_ta<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    Ok(EvalOut::Owned(match expr {
        Atr {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::atr(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        Natr {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::natr(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        WillR {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::willr(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        Cci {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::cci(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        StochK {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::stoch_k(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        StochD {
            high,
            low,
            close,
            n,
            d,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::stoch_d(h.as_panel(), l.as_panel(), c.as_panel(), *n, *d)
        }
        AroonUp { high, n } => ta::aroon_up(eval_out(high, ctx)?.as_panel(), *n),
        AroonDown { low, n } => ta::aroon_down(eval_out(low, ctx)?.as_panel(), *n),
        Adx {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::adx(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        PlusDi {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::plus_di(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        MinusDi {
            high,
            low,
            close,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            ta::minus_di(h.as_panel(), l.as_panel(), c.as_panel(), *n)
        }
        Obv { close, volume } => {
            let c = eval_out(close, ctx)?;
            let v = eval_out(volume, ctx)?;
            ta::obv(c.as_panel(), v.as_panel())
        }
        Mfi {
            high,
            low,
            close,
            volume,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            let v = eval_out(volume, ctx)?;
            ta::mfi(h.as_panel(), l.as_panel(), c.as_panel(), v.as_panel(), *n)
        }
        Vwap {
            high,
            low,
            close,
            volume,
            n,
        } => {
            let h = eval_out(high, ctx)?;
            let l = eval_out(low, ctx)?;
            let c = eval_out(close, ctx)?;
            let v = eval_out(volume, ctx)?;
            ta::vwap(h.as_panel(), l.as_panel(), c.as_panel(), v.as_panel(), *n)
        }
        _ => unreachable!("eval_ta: not a multi-series TA op"),
    }))
}

fn eval_cs<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    Ok(EvalOut::Owned(match expr {
        IsLargest { of, n } => eval_out(of, ctx)?.as_panel().is_largest(*n),
        IsSmallest { of, n } => eval_out(of, ctx)?.as_panel().is_smallest(*n),
        Sustain {
            of,
            nwindow,
            nsatisfy,
        } => eval_out(of, ctx)?.as_panel().sustain(*nwindow, *nsatisfy),
        IsEntry { of } => eval_out(of, ctx)?.as_panel().is_entry(),
        IsExit { of } => eval_out(of, ctx)?.as_panel().is_exit(),
        ExitWhen { entry, exit } => {
            let e = eval_out(entry, ctx)?;
            let x = eval_out(exit, ctx)?;
            e.as_panel().exit_when(x.as_panel())
        }
        QuantileRow { of, c } => eval_out(of, ctx)?.as_panel().quantile_row(*c),
        Winsorize { of, lower, upper } => eval_out(of, ctx)?.as_panel().winsorize(*lower, *upper),
        Zscore { of } => eval_out(of, ctx)?.as_panel().zscore(),
        Bucket { of, n } => eval_out(of, ctx)?.as_panel().bucket(*n),
        Demean { of } => eval_out(of, ctx)?.as_panel().demean(),
        Rank { of, pct, ascending } => eval_out(of, ctx)?.as_panel().rank_cs(*pct, *ascending),
        _ => unreachable!("eval_cs: not a cross-section op"),
    }))
}

fn eval_arith<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    Ok(match expr {
        Gt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x > y))?,
        Lt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x < y))?,
        Ge { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x >= y))?,
        Le { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x <= y))?,
        And { l, r } => {
            let left = eval_out(l, ctx)?;
            let right = eval_out(r, ctx)?;
            EvalOut::Owned(left.as_panel().and(right.as_panel()))
        }
        Or { l, r } => {
            let left = eval_out(l, ctx)?;
            let right = eval_out(r, ctx)?;
            EvalOut::Owned(left.as_panel().or(right.as_panel()))
        }
        Not { of } => EvalOut::Owned(eval_out(of, ctx)?.as_panel().not()),
        Add { l, r } => num_binop(l, r, ctx, |x, y| x + y)?,
        Sub { l, r } => num_binop(l, r, ctx, |x, y| x - y)?,
        Mul { l, r } => num_binop(l, r, ctx, |x, y| x * y)?,
        Div { l, r } => num_binop(l, r, ctx, |x, y| x / y)?,
        Neg { of } => EvalOut::Owned(eval_out(of, ctx)?.as_panel().neg()),
        Ceil { of } => EvalOut::Owned(eval_out(of, ctx)?.as_panel().ceil()),
        Mask { of, by } => {
            let o = eval_out(of, ctx)?;
            let b = eval_out(by, ctx)?;
            EvalOut::Owned(o.as_panel().mask(b.as_panel()))
        }
        _ => unreachable!("eval_arith: not an arithmetic/logic op"),
    })
}

fn eval_portfolio<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    match expr {
        NormalizeRow { of } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?.as_panel().normalize_row(),
        )),
        VolTarget {
            of,
            prices,
            target,
            n,
        } => {
            let signal = eval_out(of, ctx)?;
            let px = eval_out(prices, ctx)?;
            Ok(EvalOut::Owned(signal.as_panel().vol_target(
                px.as_panel(),
                *target,
                *n,
            )))
        }
        HoldUntil {
            entry,
            exit,
            nstocks_limit,
            rank,
        } => {
            let e = eval_out(entry, ctx)?;
            let x = eval_out(exit, ctx)?;
            // HoldUntilOpts stores an owned rank panel.
            let rank_panel = match rank {
                Some(r) => Some(eval_out(r, ctx)?.into_owned()),
                None => None,
            };
            let opts = HoldUntilOpts {
                nstocks_limit: *nstocks_limit,
                rank: rank_panel,
            };
            Ok(EvalOut::Owned(e.as_panel().hold_until(x.as_panel(), &opts)))
        }
        Rebalance { of, freq, on } => {
            let p = eval_out(of, ctx)?;
            match (freq.as_deref(), on) {
                (Some(_), Some(_)) => Err(EngineError::RebalanceBoth),
                (None, None) => Err(EngineError::RebalanceNeither),
                (Some(freq), None) => {
                    let f = match freq {
                        "W" => Freq::Weekly,
                        "ME" => Freq::MonthEnd,
                        "QE" => Freq::QuarterEnd,
                        "YE" => Freq::YearEnd,
                        other => {
                            return Err(EngineError::BadFreq {
                                freq: other.to_string(),
                            });
                        }
                    };
                    Ok(EvalOut::Owned(p.as_panel().rebalance_freq(f)))
                }
                (None, Some(on)) => {
                    let trigger = eval_out(on, ctx)?;
                    // Dates (already sorted, unique) where any cell in the row is
                    // true: finite and != 0. Union across symbols → portfolio
                    // rebalances whenever any holding's trigger fires.
                    let t = trigger.as_panel();
                    let dates: Vec<i32> = t
                        .dates
                        .iter()
                        .enumerate()
                        .filter(|&(r, _)| {
                            (0..t.ncols()).any(|cc| {
                                let v = t.data[[r, cc]];
                                v.is_finite() && v != 0.0
                            })
                        })
                        .map(|(_, &d)| d)
                        .collect();
                    Ok(EvalOut::Owned(p.as_panel().rebalance_dates(&dates)))
                }
            }
        }
        _ => unreachable!("eval_portfolio: not a portfolio op"),
    }
}

fn eval_industry<'a>(expr: &Expr, ctx: &'a EvalContext) -> Result<EvalOut<'a>, EngineError> {
    use Expr::*;
    match expr {
        Neutralize { of, by, add_const } => {
            let factor = eval_out(of, ctx)?;
            // neutralize takes &[Panel]; materialize owned copies once at the call.
            let neutralizers = by
                .iter()
                .map(|b| eval_out(b, ctx).map(EvalOut::into_owned))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(EvalOut::Owned(
                factor.as_panel().neutralize(&neutralizers, *add_const),
            ))
        }
        NeutralizeIndustry { of, add_const } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?
                .as_panel()
                .neutralize_industry(&ctx.industry, *add_const),
        )),
        IndustryRank { of, categories } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?
                .as_panel()
                .industry_rank(&ctx.industry, categories.as_deref()),
        )),
        CapIndustry { of, max_weight } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?
                .as_panel()
                .cap_industry(&ctx.industry, *max_weight),
        )),
        GroupbyCategory { of, agg } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?
                .as_panel()
                .groupby_category(&ctx.industry, agg)?,
        )),
        InSector { of, name } => Ok(EvalOut::Owned(
            eval_out(of, ctx)?.as_panel().in_sector(&ctx.industry, name),
        )),
        _ => unreachable!("eval_industry: not an industry op"),
    }
}

use crate::backtest::BacktestConfig;
use crate::report::{benchmark_equity, build_report_with_benchmark, Report};

/// Fixed seed for the report bootstrap — same input, same bands, every run.
const BOOTSTRAP_SEED: u64 = 0x00C1_7A05;

pub fn run_backtest(
    spec_json: &str,
    ctx: &EvalContext,
    price_key: &str,
    cfg: &BacktestConfig,
) -> Result<Report, EngineError> {
    let positions = run_strategy(spec_json, ctx)?;
    let prices = ctx
        .panels
        .get(price_key)
        .ok_or_else(|| EngineError::UnknownPriceKey {
            key: price_key.to_string(),
        })?;
    let open = ctx.panels.get("open");
    let high = ctx.panels.get("high");
    let low = ctx.panels.get("low");
    let volume = ctx.panels.get("volume");
    // `open` feeds the execution-layer stops' gap fills; `run` ignores it when
    // stops are off, so the default path is unchanged.
    let out =
        crate::backtest::run_with_initial(&positions, prices, open, high, low, volume, cfg, None);
    let benchmark = match &cfg.benchmark_key {
        Some(key) => {
            let p = ctx
                .panels
                .get(key)
                .ok_or_else(|| EngineError::UnknownBenchmark {
                    key: key.clone(),
                })?;
            if p.ncols() == 0 {
                return Err(EngineError::EmptyBenchmark { key: key.clone() });
            }
            // first column is the benchmark instrument
            let col: Vec<f64> = (0..p.nrows()).map(|r| p.data[[r, 0]]).collect();
            Some(benchmark_equity(&out.dates, &p.dates, &col))
        }
        None => None,
    };
    let mut report = build_report_with_benchmark(out, benchmark);
    if cfg.bootstrap_samples > 0 {
        report.bootstrap = crate::bootstrap::bootstrap(
            &report.dates,
            &report.equity,
            cfg.bootstrap_samples,
            cfg.bootstrap_block,
            BOOTSTRAP_SEED,
        );
    }
    if let Some(live_start) = cfg.live_performance_start {
        report.live = crate::report::live_segment(&report.dates, &report.equity, live_start);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    fn ctx() -> EvalContext {
        let close = Panel::new(
            vec![20240102, 20240103, 20240104],
            vec!["A".into(), "B".into()],
            array![[1.0, 4.0], [2.0, 3.0], [3.0, 2.0]],
        )
        .unwrap();
        EvalContext::new(HashMap::from([("close".to_string(), close)]))
    }

    #[test]
    fn run_backtest_with_benchmark_key_reports_relative_metrics() {
        let close = Panel::new(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            array![[10.0], [11.0], [12.0]],
        )
        .unwrap();
        let spy = Panel::new(
            vec![20240102, 20240103, 20240104],
            vec!["SPY".into()],
            array![[100.0], [101.0], [102.0]],
        )
        .unwrap();
        let ctx = EvalContext::new(HashMap::from([
            ("close".to_string(), close),
            ("benchmark".to_string(), spy),
        ]));
        let spec = r#"{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":0.0}}"#;
        let cfg = BacktestConfig {
            benchmark_key: Some("benchmark".to_string()),
            ..Default::default()
        };
        let report = run_backtest(spec, &ctx, "close", &cfg).unwrap();
        let bench = report.benchmark.as_ref().unwrap();
        assert_eq!(bench.len(), 3);
        assert!((bench[2] - 1.02).abs() < 1e-12);
        let m = &report.metrics;
        assert!((m.benchmark_return.unwrap() - 0.02).abs() < 1e-12);
        assert!((m.excess_return.unwrap() - (0.2 - 0.02)).abs() < 1e-9);
        assert!(m.beta.is_some() && m.alpha.is_some());

        // Unknown benchmark key errors instead of silently dropping it.
        let bad = BacktestConfig {
            benchmark_key: Some("nope".to_string()),
            ..Default::default()
        };
        assert!(run_backtest(spec, &ctx, "close", &bad).is_err());

        // bootstrap_samples > 0 attaches confidence bands to the report.
        let boot = BacktestConfig {
            bootstrap_samples: 50,
            ..Default::default()
        };
        let r = run_backtest(spec, &ctx, "close", &boot).unwrap();
        let b = r.bootstrap.as_ref().unwrap();
        assert_eq!(b.n_samples, 50);
        assert!(b.sharpe.p05 <= b.sharpe.p95);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"bootstrap\""));

        // live_performance_start attaches a post-live segment block.
        let live = BacktestConfig {
            live_performance_start: Some(20240103),
            ..Default::default()
        };
        let r = run_backtest(spec, &ctx, "close", &live).unwrap();
        let seg = r.live.as_ref().unwrap();
        assert_eq!(seg.start, 20240103);
        assert_eq!(seg.days, 2);
        assert!(serde_json::to_string(&r).unwrap().contains("\"live\""));
        // Unset -> no live block.
        let none = run_backtest(spec, &ctx, "close", &BacktestConfig::default()).unwrap();
        assert!(none.live.is_none());
    }

    #[test]
    fn rebalance_ye_keeps_last_obs_per_year() {
        let x = Panel::new(
            vec![20230103, 20231229, 20240102, 20241231],
            vec!["A".into()],
            array![[1.0], [2.0], [3.0], [4.0]],
        )
        .unwrap();
        let c = EvalContext::new(HashMap::from([("x".to_string(), x)]));
        let spec = r#"{"op":"Rebalance","of":{"op":"Data","name":"x"},"freq":"YE"}"#;
        let got = run_strategy(spec, &c).unwrap();
        assert_eq!(got.dates, vec![20231229, 20241231]);
        assert_eq!(got.data[[0, 0]], 2.0);
        assert_eq!(got.data[[1, 0]], 4.0);
    }

    #[test]
    fn normalize_row_makes_explicit_weights() {
        // inverse-vol-style weighting: normalize_row over a raw signal
        let sig = Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into()],
            array![[1.0, 3.0]],
        )
        .unwrap();
        let c = EvalContext::new(HashMap::from([("sig".to_string(), sig)]));
        let spec = r#"{"op":"NormalizeRow","of":{"op":"Data","name":"sig"}}"#;
        let got = run_strategy(spec, &c).unwrap();
        assert_eq!(got.data[[0, 0]], 0.25);
        assert_eq!(got.data[[0, 1]], 0.75);
        // and the lemon surface parses to the same tree
        let parsed = lemon::parse("normalize_row(sig)").unwrap();
        assert_eq!(parsed["op"], "NormalizeRow");
    }

    #[test]
    fn not_inverts_booleans_with_nan_falsy() {
        let sig = Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into()],
            array![[1.0, 0.0, f64::NAN]],
        )
        .unwrap();
        let c = EvalContext::new(HashMap::from([("sig".to_string(), sig)]));
        let spec = r#"{"op":"Not","of":{"op":"Data","name":"sig"}}"#;
        let got = run_strategy(spec, &c).unwrap();
        assert_eq!(got.data[[0, 0]], 0.0); // not true
        assert_eq!(got.data[[0, 1]], 1.0); // not false
        assert_eq!(got.data[[0, 2]], 1.0); // NaN is falsy -> not NaN = 1
    }

    #[test]
    fn evaluates_nested_indicator_and_logical_spec() {
        // is_largest(close, 1) AND (close rise 1) — exercises Data/IsLargest/Rise/And.
        let spec = r#"{
            "op": "And",
            "l": { "op": "IsLargest", "of": { "op": "Data", "name": "close" }, "n": 1 },
            "r": { "op": "Rise", "of": { "op": "Data", "name": "close" }, "n": 1 }
        }"#;
        let got = run_strategy(spec, &ctx()).unwrap();
        // row 2: A is largest? A=3,B=2 -> A largest; A rose (3>2) -> A true, B false.
        assert_eq!(got.data[[2, 0]], 1.0);
        assert_eq!(got.data[[2, 1]], 0.0);
    }

    #[test]
    fn exit_when_and_quantile_row_via_spec() {
        // entry: close > 1.5 (false,true,true on A=1,2,3), exit: always false.
        // Hold from first entry edge (row1) onward for A; B never enters (0.5,1,2 all?).
        // B: 0.5,1,2 — >1.5 only row2; entry edge at row2.
        let entry =
            r#"{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":1.5}}"#;
        let exit = r#"{"op":"Lt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":0.0}}"#;
        let ew = run_strategy(
            &format!(r#"{{"op":"ExitWhen","entry":{entry},"exit":{exit}}}"#),
            &ctx(),
        )
        .unwrap();
        assert_eq!(ew.data[[0, 0]], 0.0);
        assert_eq!(ew.data[[1, 0]], 1.0);
        assert_eq!(ew.data[[2, 0]], 1.0);

        let q = run_strategy(
            r#"{"op":"QuantileRow","of":{"op":"Data","name":"close"},"c":0.5}"#,
            &ctx(),
        )
        .unwrap();
        assert_eq!(q.ncols(), 1);
        // row0: A=1,B=4 → median 2.5 (linear interp)
        assert!((q.data[[0, 0]] - 2.5).abs() < 1e-12);
    }

    #[test]
    fn in_sector_via_spec() {
        let close = Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into()],
            array![[10.0, 20.0]],
        )
        .unwrap();
        let mut industry = HashMap::new();
        industry.insert("A".into(), "Technology".into());
        industry.insert("B".into(), "Energy".into());
        let c = EvalContext {
            panels: HashMap::from([("close".to_string(), close)]),
            industry,
        };
        let got = run_strategy(
            r#"{"op":"InSector","of":{"op":"Data","name":"close"},"name":"Technology"}"#,
            &c,
        )
        .unwrap();
        assert_eq!(got.data[[0, 0]], 1.0);
        assert_eq!(got.data[[0, 1]], 0.0);

        let masked = run_strategy(
            r#"{
              "op":"Mask",
              "of":{"op":"Data","name":"close"},
              "by":{"op":"InSector","of":{"op":"Data","name":"close"},"name":"Technology"}
            }"#,
            &c,
        )
        .unwrap();
        assert_eq!(masked.data[[0, 0]], 10.0);
        assert!(masked.data[[0, 1]].is_nan());
    }

    #[test]
    fn cs_preprocess_ops_via_spec() {
        let dem = run_strategy(
            r#"{"op":"Demean","of":{"op":"Data","name":"close"}}"#,
            &ctx(),
        )
        .unwrap();
        // row0: 1,4 mean 2.5 → -1.5, 1.5
        assert!((dem.data[[0, 0]] + 1.5).abs() < 1e-12);
        assert!((dem.data[[0, 1]] - 1.5).abs() < 1e-12);

        let z = run_strategy(
            r#"{"op":"Zscore","of":{"op":"Data","name":"close"}}"#,
            &ctx(),
        )
        .unwrap();
        assert!(z.data[[0, 0]].is_finite());

        let w = run_strategy(
            r#"{"op":"Winsorize","of":{"op":"Data","name":"close"},"lower":0.0,"upper":1.0}"#,
            &ctx(),
        )
        .unwrap();
        assert_eq!(w.data[[0, 0]], 1.0);

        let b = run_strategy(
            r#"{"op":"Bucket","of":{"op":"Data","name":"close"},"n":2}"#,
            &ctx(),
        )
        .unwrap();
        // two symbols → buckets 1 and 2
        assert_eq!(b.data[[0, 0]], 1.0);
        assert_eq!(b.data[[0, 1]], 2.0);
    }

    #[test]
    fn macd_and_bollinger_compose_from_primitives() {
        let c = ctx();
        let d = r#"{"op":"Data","name":"close"}"#;
        // MACD line = Ema(close,1) - Ema(close,2). Ema(_,1) with k=1 is the
        // series itself, so this just checks the composition parses + runs.
        let macd = run_strategy(
            &format!(
                r#"{{"op":"Sub","l":{{"op":"Ema","of":{d},"n":1}},"r":{{"op":"Ema","of":{d},"n":2}}}}"#
            ),
            &c,
        );
        assert!(macd.is_ok());
        // Bollinger upper band = Average(close,2) + 2*Std(close,2).
        let boll = run_strategy(
            &format!(
                r#"{{"op":"Add","l":{{"op":"Average","of":{d},"n":2}},"r":{{"op":"Mul","l":{{"op":"Std","of":{d},"n":2}},"r":{{"op":"Const","value":2.0}}}}}}"#
            ),
            &c,
        );
        assert!(boll.is_ok());
    }

    #[test]
    fn named_ta_indicators_evaluate() {
        let c = ctx(); // close col A = [1,2,3]
        let d = r#"{"op":"Data","name":"close"}"#;

        // rolling_min mirrors rolling_max: n=2 over [1,2,3] -> row1=1, row2=2.
        let rmin = run_strategy(&format!(r#"{{"op":"RollingMin","of":{d},"n":2}}"#), &c).unwrap();
        assert!(rmin.data[[0, 0]].is_nan());
        assert_eq!(rmin.data[[1, 0]], 1.0);
        assert_eq!(rmin.data[[2, 0]], 2.0);

        // Donchian: high=rolling_max, low=rolling_min, mid=(hi+lo)/2. Row2: (3+2)/2.
        let dmid = run_strategy(&format!(r#"{{"op":"DonchianMid","of":{d},"n":2}}"#), &c).unwrap();
        assert_eq!(dmid.data[[2, 0]], 2.5);

        // Bollinger upper with defaulted k=2: average(2)=[1,1.5,2.5], std(2)=[_,0.5,0.5].
        let up = run_strategy(&format!(r#"{{"op":"BollingerUpper","of":{d},"n":2}}"#), &c).unwrap();
        assert!(up.data[[0, 0]].is_nan()); // std warm-up
        assert!((up.data[[1, 0]] - 2.5).abs() < 1e-12);
        assert!((up.data[[2, 0]] - 3.5).abs() < 1e-12);

        // MACD(fast=1, slow=2): ema(1)=[1,2,3], ema(2)=[_,1.5,2.5] -> macd=[_,0.5,0.5].
        let macd = run_strategy(
            &format!(r#"{{"op":"Macd","of":{d},"fast":1,"slow":2}}"#),
            &c,
        )
        .unwrap();
        assert!((macd.data[[1, 0]] - 0.5).abs() < 1e-12);
        assert!((macd.data[[2, 0]] - 0.5).abs() < 1e-12);

        // Defaulted params (fast/slow/signal = 12/26/9) parse and run (all warm-up NaN here).
        for op in [
            "Macd",
            "MacdSignal",
            "MacdHist",
            "BollingerLower",
            "DonchianHigh",
        ] {
            let spec = if op.starts_with("Bollinger") || op.starts_with("Donchian") {
                format!(r#"{{"op":"{op}","of":{d},"n":3}}"#)
            } else {
                format!(r#"{{"op":"{op}","of":{d}}}"#)
            };
            assert!(run_strategy(&spec, &c).is_ok(), "{op} failed to evaluate");
        }
    }

    #[test]
    fn rsi_variant_parses_and_runs() {
        // close column A = [1,2,3] strictly rising over 3 rows; rsi(1) => first
        // finite at row 1, value 100 (no losses).
        let c = ctx();
        let r = run_strategy(
            r#"{"op":"Rsi","of":{"op":"Data","name":"close"},"n":1}"#,
            &c,
        )
        .unwrap();
        // ctx close col A = [1,2,3] -> rising -> 100 from row 1 on.
        assert_eq!(r.data[[1, 0]], 100.0);
        assert_eq!(r.data[[2, 0]], 100.0);
    }

    #[test]
    fn scalar_broadcasts_on_either_side() {
        let c = ctx();
        // close > 2 (scalar rhs): row2 [3,2] => [1,0]
        let gt = run_strategy(
            r#"{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":2.0}}"#,
            &c,
        )
        .unwrap();
        assert_eq!(gt.data[[2, 0]], 1.0);
        assert_eq!(gt.data[[2, 1]], 0.0);
        // 6 / close (scalar lhs, non-commutative): row0 [1,4] => [6,1.5]
        let div = run_strategy(
            r#"{"op":"Div","l":{"op":"Const","value":6.0},"r":{"op":"Data","name":"close"}}"#,
            &c,
        )
        .unwrap();
        assert_eq!(div.data[[0, 0]], 6.0);
        assert_eq!(div.data[[0, 1]], 1.5);
    }

    #[test]
    fn rank_ceil_mask_add_mul_evaluate() {
        let c = ctx();
        let d = r#"{"op":"Data","name":"close"}"#;
        // Rank pct ascending, row0 [1,4] => A=0.5, B=1.0
        let rank = run_strategy(
            &format!(r#"{{"op":"Rank","of":{d},"pct":true,"ascending":true}}"#),
            &c,
        )
        .unwrap();
        assert_eq!(rank.data[[0, 0]], 0.5);
        assert_eq!(rank.data[[0, 1]], 1.0);
        // Add close + close, row0 [1,4] => [2,8]
        let add = run_strategy(&format!(r#"{{"op":"Add","l":{d},"r":{d}}}"#), &c).unwrap();
        assert_eq!(add.data[[0, 0]], 2.0);
        assert_eq!(add.data[[0, 1]], 8.0);
        // Ceil(close * 0.5), row1 [2,3] => [1.0,1.5] => [1,2]
        let ceil = run_strategy(
            &format!(
                r#"{{"op":"Ceil","of":{{"op":"Mul","l":{d},"r":{{"op":"Const","value":0.5}}}}}}"#
            ),
            &c,
        )
        .unwrap();
        assert_eq!(ceil.data[[1, 0]], 1.0);
        assert_eq!(ceil.data[[1, 1]], 2.0);
        // Mask close by (close > 2), row2 [3,2] => [3, NaN]
        let mask = run_strategy(
            &format!(
                r#"{{"op":"Mask","of":{d},"by":{{"op":"Gt","l":{d},"r":{{"op":"Const","value":2.0}}}}}}"#
            ),
            &c,
        )
        .unwrap();
        assert_eq!(mask.data[[2, 0]], 3.0);
        assert!(mask.data[[2, 1]].is_nan());
    }

    #[test]
    fn rebalance_variant_parses_and_runs() {
        let spec = r#"{ "op": "Rebalance",
            "of": { "op": "Data", "name": "close" }, "freq": "W" }"#;
        assert!(run_strategy(spec, &ctx()).is_ok());
    }

    #[test]
    fn unknown_series_errors() {
        let spec = r#"{ "op": "Data", "name": "missing" }"#;
        assert!(run_strategy(spec, &ctx()).is_err());
    }

    #[test]
    fn bare_const_rejected() {
        let spec = r#"{ "op": "Const", "value": 1.0 }"#;
        assert!(run_strategy(spec, &ctx()).is_err());
    }

    #[test]
    fn bad_freq_errors() {
        let spec = r#"{ "op": "Rebalance",
            "of": { "op": "Data", "name": "close" }, "freq": "Z" }"#;
        assert!(run_strategy(spec, &ctx()).is_err());
    }

    #[test]
    fn malformed_json_errors() {
        assert!(run_strategy("{ not json", &ctx()).is_err());
    }

    #[test]
    fn rebalance_on_series_samples_trigger_dates() {
        // trigger true only on row 0 (date 20240102): rebalance there only.
        let c = ctx();
        let spec = r#"{
            "op": "Rebalance",
            "of": { "op": "Data", "name": "close" },
            "on": { "op": "Gt", "l": { "op": "Data", "name": "close" }, "r": { "op": "Const", "value": 3.5 } }
        }"#;
        // close > 3.5 is true at row0 colB (4>3.5) only -> trigger dates = [20240102].
        let r = run_strategy(spec, &c).unwrap();
        assert_eq!(r.dates, vec![20240102]);
        assert_eq!(r.data[[0, 0]], 1.0); // sampled the close row at that date
    }

    #[test]
    fn rebalance_rejects_both_and_neither() {
        let c = ctx();
        let d = r#"{"op":"Data","name":"close"}"#;
        let both = format!(r#"{{"op":"Rebalance","of":{d},"freq":"W","on":{d}}}"#);
        let neither = format!(r#"{{"op":"Rebalance","of":{d}}}"#);
        assert!(run_strategy(&both, &c).is_err());
        assert!(run_strategy(&neither, &c).is_err());
    }

    #[test]
    fn shift_variant_lags_the_series() {
        // ctx close col A = [1,2,3]; shift(1) -> [NaN,1,2]
        let c = ctx();
        let r = run_strategy(
            r#"{"op":"Shift","of":{"op":"Data","name":"close"},"n":1}"#,
            &c,
        )
        .unwrap();
        assert!(r.data[[0, 0]].is_nan());
        assert_eq!(r.data[[1, 0]], 1.0);
        assert_eq!(r.data[[2, 0]], 2.0);
    }

    #[test]
    fn rolling_max_variant_evaluates() {
        let c = ctx();
        // ctx close col A = [1,2,3]; rolling_max(2) -> [NaN,2,3]
        let r = run_strategy(
            r#"{"op":"RollingMax","of":{"op":"Data","name":"close"},"n":2}"#,
            &c,
        )
        .unwrap();
        assert!(r.data[[0, 0]].is_nan());
        assert_eq!(r.data[[1, 0]], 2.0);
        assert_eq!(r.data[[2, 0]], 3.0);
    }

    fn ohlc_ctx() -> EvalContext {
        let mk = |a: f64, b: f64, c2: f64, d: f64, e: f64, f: f64| {
            Panel::from_rows(
                (0..6).map(|i| 20240102 + i).collect(),
                vec!["A".into()],
                vec![vec![a], vec![b], vec![c2], vec![d], vec![e], vec![f]],
            )
            .unwrap()
        };
        EvalContext::new(HashMap::from([
            ("high".into(), mk(10.0, 11.0, 12.0, 11.0, 13.0, 12.0)),
            ("low".into(), mk(8.0, 9.0, 10.0, 9.0, 10.0, 11.0)),
            ("close".into(), mk(9.0, 10.0, 11.0, 10.0, 12.0, 11.0)),
            (
                "volume".into(),
                mk(100.0, 110.0, 120.0, 130.0, 140.0, 150.0),
            ),
        ]))
    }

    #[test]
    fn atr_variant_evaluates() {
        let spec = r#"{"op":"Atr",
            "high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},
            "close":{"op":"Data","name":"close"},"n":3}"#;
        let r = run_strategy(spec, &ohlc_ctx()).unwrap();
        assert!(r.data[[2, 0]].is_nan());
        assert!((r.data[[3, 0]] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn aroon_variants_evaluate() {
        for (op, arg) in [("AroonUp", "high"), ("AroonDown", "low")] {
            let spec = format!(r#"{{"op":"{op}","{arg}":{{"op":"Data","name":"{arg}"}},"n":3}}"#);
            assert!(run_strategy(&spec, &ohlc_ctx()).is_ok(), "failed: {op}");
        }
    }

    #[test]
    fn cci_variant_evaluates() {
        let spec = r#"{"op":"Cci","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},
            "close":{"op":"Data","name":"close"},"n":3}"#;
        let r = run_strategy(spec, &ohlc_ctx()).unwrap();
        assert!((r.data[[2, 0]] - 100.0).abs() < 1e-6);
    }

    #[test]
    fn natr_willr_variants_evaluate() {
        let ops = ["Natr", "WillR"];
        for op in ops {
            let spec = format!(
                r#"{{"op":"{op}","high":{{"op":"Data","name":"high"}},
                    "low":{{"op":"Data","name":"low"}},
                    "close":{{"op":"Data","name":"close"}},"n":3}}"#
            );
            assert!(run_strategy(&spec, &ohlc_ctx()).is_ok(), "failed: {op}");
        }
    }

    #[test]
    fn stoch_variants_evaluate() {
        let k = r#"{"op":"StochK","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},"close":{"op":"Data","name":"close"},"n":3}"#;
        assert!(run_strategy(k, &ohlc_ctx()).is_ok());
        // %D with d omitted defaults to 3.
        let d = r#"{"op":"StochD","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},"close":{"op":"Data","name":"close"},"n":3}"#;
        let r = run_strategy(d, &ohlc_ctx()).unwrap();
        assert!((r.data[[4, 0]] - 550.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn adx_di_variants_evaluate() {
        for op in ["Adx", "PlusDi", "MinusDi"] {
            let spec = format!(
                r#"{{"op":"{op}","high":{{"op":"Data","name":"high"}},
                    "low":{{"op":"Data","name":"low"}},
                    "close":{{"op":"Data","name":"close"}},"n":2}}"#
            );
            assert!(run_strategy(&spec, &ohlc_ctx()).is_ok(), "failed: {op}");
        }
    }

    #[test]
    fn obv_mfi_variants_evaluate() {
        let obv = r#"{"op":"Obv","close":{"op":"Data","name":"close"},
            "volume":{"op":"Data","name":"volume"}}"#;
        assert!(run_strategy(obv, &ohlc_ctx()).is_ok());
        let mfi = r#"{"op":"Mfi","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},"close":{"op":"Data","name":"close"},
            "volume":{"op":"Data","name":"volume"},"n":3}"#;
        assert!(run_strategy(mfi, &ohlc_ctx()).is_ok());
    }

    #[test]
    fn vwap_variant_evaluates() {
        let spec = r#"{"op":"Vwap","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},"close":{"op":"Data","name":"close"},
            "volume":{"op":"Data","name":"volume"},"n":3}"#;
        assert!(run_strategy(spec, &ohlc_ctx()).is_ok());
    }

    // `hold_until` no longer carries price stops — they moved to
    // `BacktestConfig::stops` (execution layer). Stop behavior is tested in
    // `backtest.rs`; the old op-level stop tests are retired.

    #[test]
    fn all_remaining_variants_evaluate() {
        // One spec per otherwise-uncovered Expr arm — guards against a serde tag
        // typo or a wrong method wiring in any single variant.
        let d = r#"{"op":"Data","name":"close"}"#;
        let rise = format!(r#"{{"op":"Rise","of":{d},"n":1}}"#);
        let specs = [
            format!(r#"{{"op":"Fall","of":{d},"n":1}}"#),
            format!(r#"{{"op":"IsSmallest","of":{d},"n":1}}"#),
            format!(r#"{{"op":"Sustain","of":{rise},"nwindow":2,"nsatisfy":null}}"#),
            format!(r#"{{"op":"IsEntry","of":{rise}}}"#),
            format!(r#"{{"op":"IsExit","of":{rise}}}"#),
            format!(r#"{{"op":"Ge","l":{d},"r":{d}}}"#),
            format!(r#"{{"op":"Le","l":{d},"r":{d}}}"#),
            format!(r#"{{"op":"Or","l":{rise},"r":{rise}}}"#),
            format!(r#"{{"op":"Sub","l":{d},"r":{d}}}"#),
            format!(r#"{{"op":"Div","l":{d},"r":{d}}}"#),
            format!(r#"{{"op":"Neg","of":{d}}}"#),
            format!(r#"{{"op":"Neutralize","of":{d},"by":[{d}],"add_const":true}}"#),
            format!(r#"{{"op":"NeutralizeIndustry","of":{d},"add_const":true}}"#),
            format!(r#"{{"op":"IndustryRank","of":{d},"categories":null}}"#),
            format!(r#"{{"op":"CapIndustry","of":{d},"max_weight":0.3}}"#),
            format!(r#"{{"op":"VolTarget","of":{d},"prices":{d},"target":0.1,"n":63}}"#),
            format!(r#"{{"op":"GroupbyCategory","of":{d},"agg":"mean"}}"#),
        ];
        for s in &specs {
            assert!(run_strategy(s, &ctx()).is_ok(), "failed to evaluate: {s}");
        }
    }

    /// `Data` leaves used only as intermediates must not force a deep clone.
    /// Before EvalOut, each hit cloned the full panel; after, hits ≫ clones.
    #[test]
    fn data_leaves_borrow_without_deep_clone() {
        let _ = take_eval_counters(); // reset
                                      // close > average(close, 2): two Data leaves, root is computed Owned.
        let spec = r#"{
            "op":"Gt",
            "l":{"op":"Data","name":"close"},
            "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":2}
        }"#;
        let got = run_strategy(spec, &ctx()).unwrap();
        assert_eq!(got.nrows(), 3);
        let (hits, clones) = take_eval_counters();
        assert_eq!(hits, 2, "two Data leaves visited");
        assert_eq!(
            clones, 0,
            "no borrowed→owned deep clone when root is computed"
        );

        // Bare Data still materializes once at the public boundary.
        let bare = run_strategy(r#"{"op":"Data","name":"close"}"#, &ctx()).unwrap();
        assert_eq!(bare.nrows(), 3);
        let (hits, clones) = take_eval_counters();
        assert_eq!(hits, 1);
        assert_eq!(clones, 1, "root Data must clone once for owned return");

        // Five references to the same series still zero intermediate clones.
        let d = r#"{"op":"Data","name":"close"}"#;
        let five = format!(
            r#"{{"op":"Add","l":{{"op":"Add","l":{{"op":"Add","l":{{"op":"Add","l":{d},"r":{d}}},"r":{d}}},"r":{d}}},"r":{d}}}"#
        );
        let sum = run_strategy(&five, &ctx()).unwrap();
        assert_eq!(sum.data[[0, 0]], 5.0); // 1+1+1+1+1
        let (hits, clones) = take_eval_counters();
        assert_eq!(hits, 5);
        assert_eq!(clones, 0);
    }

    /// Smoke timing on a large-ish panel (T=500, S=200). Documents wall-clock
    /// for multi-`close` eval; primary win is clone count, not necessarily latency.
    #[test]
    fn large_panel_multi_close_eval_smoke() {
        use std::time::Instant;
        let t = 500usize;
        let s = 200usize;
        let dates: Vec<i32> = (0..t as i32).map(|i| 20200101 + i).collect();
        let symbols: Vec<String> = (0..s).map(|i| format!("S{i:04}")).collect();
        let data = ndarray::Array2::from_shape_fn((t, s), |(r, c)| {
            100.0 + (r as f64) * 0.01 + (c as f64) * 0.001
        });
        let close = Panel::new(dates, symbols, data).unwrap();
        let c = EvalContext::new(HashMap::from([("close".to_string(), close)]));
        // close > average(close, 20) — two Data leaves + one rolling mean.
        let spec = r#"{
            "op":"Gt",
            "l":{"op":"Data","name":"close"},
            "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":20}
        }"#;
        let _ = take_eval_counters();
        let start = Instant::now();
        let iters = 20;
        for _ in 0..iters {
            let p = run_strategy(spec, &c).unwrap();
            assert_eq!(p.nrows(), t);
            assert_eq!(p.ncols(), s);
        }
        let elapsed = start.elapsed();
        let (hits, clones) = take_eval_counters();
        assert_eq!(hits, 2 * iters as u64);
        assert_eq!(clones, 0);
        eprintln!(
            "large_panel_multi_close: {iters} iters of 500×200 Gt(close, avg(close,20)) \
             in {elapsed:?} ({:.2} ms/iter); data_hits={hits} borrowed_clones={clones}",
            elapsed.as_secs_f64() * 1000.0 / iters as f64
        );
    }
}
