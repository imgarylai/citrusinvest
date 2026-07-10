//! Evaluates an [`Expr`] tree against a data context (`HashMap<String, Panel>`).
//! [`run_strategy`] parses a JSON spec and evaluates it to a position matrix.

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
        serde_json::from_str(spec_json).map_err(|e| EngineError::Eval(e.to_string()))?;
    eval(&expr, ctx)
}

/// A `Const` operand carries a scalar; anything else evaluates to a panel.
fn as_const(e: &Expr) -> Option<f64> {
    match e {
        Expr::Const { value } => Some(*value),
        _ => None,
    }
}

/// Evaluate a binary numeric/comparison op, broadcasting a `Const` on either
/// side against the other operand's panel (`panel <op> scalar`). Rejects
/// `const <op> const` — there's no panel shape to broadcast onto.
fn num_binop(
    l: &Expr,
    r: &Expr,
    ctx: &EvalContext,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Panel, EngineError> {
    match (as_const(l), as_const(r)) {
        (Some(_), Some(_)) => Err(EngineError::Eval(
            "both operands of a binary op are Const".into(),
        )),
        (Some(a), None) => Ok(eval(r, ctx)?.scalar_lhs(a, f)),
        (None, Some(b)) => Ok(eval(l, ctx)?.scalar_rhs(b, f)),
        (None, None) => Ok(eval(l, ctx)?.ewise(&eval(r, ctx)?, f)),
    }
}

/// Evaluate an expression tree. The top-level match is grouped by op family and
/// stays **exhaustive** — a new `Expr` variant that is not listed here fails to
/// compile. Family helpers also match exhaustively over their subset.
pub fn eval(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
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

fn eval_leaf(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    match expr {
        Data { name } => ctx
            .panels
            .get(name)
            .cloned()
            .ok_or_else(|| EngineError::Eval(format!("unknown series '{name}'"))),
        Const { value } => {
            // Const is only meaningful inside scalar ops, handled by callers.
            Err(EngineError::Eval(format!(
                "bare Const {value} not allowed at top level"
            )))
        }
        _ => unreachable!("eval_leaf: not a leaf"),
    }
}

fn eval_ts(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    Ok(match expr {
        Average { of, n } => eval(of, ctx)?.average(*n),
        Ema { of, n } => eval(of, ctx)?.ema(*n),
        Std { of, n } => eval(of, ctx)?.rolling_std(*n),
        Rsi { of, n } => eval(of, ctx)?.rsi(*n),
        PctChange { of, n } => eval(of, ctx)?.pct_change(*n),
        Rise { of, n } => eval(of, ctx)?.rise(*n),
        Fall { of, n } => eval(of, ctx)?.fall(*n),
        Shift { of, n } => eval(of, ctx)?.shift(*n),
        RollingMax { of, n } => eval(of, ctx)?.rolling_max(*n),
        RollingMin { of, n } => eval(of, ctx)?.rolling_min(*n),
        BollingerMid { of, n } => eval(of, ctx)?.bollinger_mid(*n),
        BollingerUpper { of, n, k } => eval(of, ctx)?.bollinger_upper(*n, *k),
        BollingerLower { of, n, k } => eval(of, ctx)?.bollinger_lower(*n, *k),
        Macd { of, fast, slow } => eval(of, ctx)?.macd(*fast, *slow),
        MacdSignal {
            of,
            fast,
            slow,
            signal,
        } => eval(of, ctx)?.macd_signal(*fast, *slow, *signal),
        MacdHist {
            of,
            fast,
            slow,
            signal,
        } => eval(of, ctx)?.macd_hist(*fast, *slow, *signal),
        DonchianHigh { of, n } => eval(of, ctx)?.donchian_high(*n),
        DonchianLow { of, n } => eval(of, ctx)?.donchian_low(*n),
        DonchianMid { of, n } => eval(of, ctx)?.donchian_mid(*n),
        _ => unreachable!("eval_ts: not a unary TS op"),
    })
}

fn eval_ta(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    Ok(match expr {
        Atr {
            high,
            low,
            close,
            n,
        } => ta::atr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        Natr {
            high,
            low,
            close,
            n,
        } => ta::natr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        WillR {
            high,
            low,
            close,
            n,
        } => ta::willr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        Cci {
            high,
            low,
            close,
            n,
        } => ta::cci(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        StochK {
            high,
            low,
            close,
            n,
        } => ta::stoch_k(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        StochD {
            high,
            low,
            close,
            n,
            d,
        } => ta::stoch_d(
            &eval(high, ctx)?,
            &eval(low, ctx)?,
            &eval(close, ctx)?,
            *n,
            *d,
        ),
        AroonUp { high, n } => ta::aroon_up(&eval(high, ctx)?, *n),
        AroonDown { low, n } => ta::aroon_down(&eval(low, ctx)?, *n),
        Adx {
            high,
            low,
            close,
            n,
        } => ta::adx(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        PlusDi {
            high,
            low,
            close,
            n,
        } => ta::plus_di(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        MinusDi {
            high,
            low,
            close,
            n,
        } => ta::minus_di(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n),
        Obv { close, volume } => ta::obv(&eval(close, ctx)?, &eval(volume, ctx)?),
        Mfi {
            high,
            low,
            close,
            volume,
            n,
        } => ta::mfi(
            &eval(high, ctx)?,
            &eval(low, ctx)?,
            &eval(close, ctx)?,
            &eval(volume, ctx)?,
            *n,
        ),
        Vwap {
            high,
            low,
            close,
            volume,
            n,
        } => ta::vwap(
            &eval(high, ctx)?,
            &eval(low, ctx)?,
            &eval(close, ctx)?,
            &eval(volume, ctx)?,
            *n,
        ),
        _ => unreachable!("eval_ta: not a multi-series TA op"),
    })
}

fn eval_cs(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    Ok(match expr {
        IsLargest { of, n } => eval(of, ctx)?.is_largest(*n),
        IsSmallest { of, n } => eval(of, ctx)?.is_smallest(*n),
        Sustain {
            of,
            nwindow,
            nsatisfy,
        } => eval(of, ctx)?.sustain(*nwindow, *nsatisfy),
        IsEntry { of } => eval(of, ctx)?.is_entry(),
        IsExit { of } => eval(of, ctx)?.is_exit(),
        ExitWhen { entry, exit } => eval(entry, ctx)?.exit_when(&eval(exit, ctx)?),
        QuantileRow { of, c } => eval(of, ctx)?.quantile_row(*c),
        Winsorize { of, lower, upper } => eval(of, ctx)?.winsorize(*lower, *upper),
        Zscore { of } => eval(of, ctx)?.zscore(),
        Bucket { of, n } => eval(of, ctx)?.bucket(*n),
        Demean { of } => eval(of, ctx)?.demean(),
        Rank { of, pct, ascending } => eval(of, ctx)?.rank_cs(*pct, *ascending),
        _ => unreachable!("eval_cs: not a cross-section op"),
    })
}

fn eval_arith(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    Ok(match expr {
        Gt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x > y))?,
        Lt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x < y))?,
        Ge { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x >= y))?,
        Le { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x <= y))?,
        And { l, r } => eval(l, ctx)?.and(&eval(r, ctx)?),
        Or { l, r } => eval(l, ctx)?.or(&eval(r, ctx)?),
        Not { of } => eval(of, ctx)?.not(),
        Add { l, r } => num_binop(l, r, ctx, |x, y| x + y)?,
        Sub { l, r } => num_binop(l, r, ctx, |x, y| x - y)?,
        Mul { l, r } => num_binop(l, r, ctx, |x, y| x * y)?,
        Div { l, r } => num_binop(l, r, ctx, |x, y| x / y)?,
        Neg { of } => eval(of, ctx)?.neg(),
        Ceil { of } => eval(of, ctx)?.ceil(),
        Mask { of, by } => eval(of, ctx)?.mask(&eval(by, ctx)?),
        _ => unreachable!("eval_arith: not an arithmetic/logic op"),
    })
}

fn eval_portfolio(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    match expr {
        NormalizeRow { of } => Ok(eval(of, ctx)?.normalize_row()),
        VolTarget {
            of,
            prices,
            target,
            n,
        } => Ok(eval(of, ctx)?.vol_target(&eval(prices, ctx)?, *target, *n)),
        HoldUntil {
            entry,
            exit,
            nstocks_limit,
            rank,
        } => {
            let e = eval(entry, ctx)?;
            let x = eval(exit, ctx)?;
            let rank_panel = match rank {
                Some(r) => Some(eval(r, ctx)?),
                None => None,
            };
            let opts = HoldUntilOpts {
                nstocks_limit: *nstocks_limit,
                rank: rank_panel,
            };
            Ok(e.hold_until(&x, &opts))
        }
        Rebalance { of, freq, on } => {
            let p = eval(of, ctx)?;
            match (freq.as_deref(), on) {
                (Some(_), Some(_)) => Err(EngineError::Eval(
                    "Rebalance takes either freq or on, not both".into(),
                )),
                (None, None) => Err(EngineError::Eval("Rebalance needs freq or on".into())),
                (Some(freq), None) => {
                    let f = match freq {
                        "W" => Freq::Weekly,
                        "ME" => Freq::MonthEnd,
                        "QE" => Freq::QuarterEnd,
                        "YE" => Freq::YearEnd,
                        other => {
                            return Err(EngineError::Eval(format!("bad freq '{other}'")));
                        }
                    };
                    Ok(p.rebalance_freq(f))
                }
                (None, Some(on)) => {
                    let trigger = eval(on, ctx)?;
                    // Dates (already sorted, unique) where any cell in the row is
                    // true: finite and != 0. Union across symbols → portfolio
                    // rebalances whenever any holding's trigger fires.
                    let dates: Vec<i32> = trigger
                        .dates
                        .iter()
                        .enumerate()
                        .filter(|&(r, _)| {
                            (0..trigger.ncols()).any(|cc| {
                                let v = trigger.data[[r, cc]];
                                v.is_finite() && v != 0.0
                            })
                        })
                        .map(|(_, &d)| d)
                        .collect();
                    Ok(p.rebalance_dates(&dates))
                }
            }
        }
        _ => unreachable!("eval_portfolio: not a portfolio op"),
    }
}

fn eval_industry(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    match expr {
        Neutralize { of, by, add_const } => {
            let factor = eval(of, ctx)?;
            let neutralizers = by
                .iter()
                .map(|b| eval(b, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(factor.neutralize(&neutralizers, *add_const))
        }
        NeutralizeIndustry { of, add_const } => {
            Ok(eval(of, ctx)?.neutralize_industry(&ctx.industry, *add_const))
        }
        IndustryRank { of, categories } => {
            Ok(eval(of, ctx)?.industry_rank(&ctx.industry, categories.as_deref()))
        }
        CapIndustry { of, max_weight } => {
            Ok(eval(of, ctx)?.cap_industry(&ctx.industry, *max_weight))
        }
        GroupbyCategory { of, agg } => eval(of, ctx)?.groupby_category(&ctx.industry, agg),
        InSector { of, name } => Ok(eval(of, ctx)?.in_sector(&ctx.industry, name)),
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
        .ok_or_else(|| EngineError::Eval(format!("unknown price series '{price_key}'")))?;
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
                .ok_or_else(|| EngineError::Eval(format!("unknown benchmark series '{key}'")))?;
            if p.ncols() == 0 {
                return Err(EngineError::Eval(format!(
                    "benchmark series '{key}' has no symbols"
                )));
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
}
