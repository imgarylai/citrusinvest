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
        EvalContext { panels, industry: HashMap::new() }
    }
}

impl From<HashMap<String, Panel>> for EvalContext {
    fn from(panels: HashMap<String, Panel>) -> Self {
        EvalContext::new(panels)
    }
}

pub fn run_strategy(spec_json: &str, ctx: &EvalContext) -> Result<Panel, EngineError> {
    let expr: Expr = serde_json::from_str(spec_json).map_err(|e| EngineError::Eval(e.to_string()))?;
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
        (Some(_), Some(_)) => {
            Err(EngineError::Eval("both operands of a binary op are Const".into()))
        }
        (Some(a), None) => Ok(eval(r, ctx)?.scalar_lhs(a, f)),
        (None, Some(b)) => Ok(eval(l, ctx)?.scalar_rhs(b, f)),
        (None, None) => Ok(eval(l, ctx)?.ewise(&eval(r, ctx)?, f)),
    }
}

pub fn eval(expr: &Expr, ctx: &EvalContext) -> Result<Panel, EngineError> {
    use Expr::*;
    Ok(match expr {
        Data { name } => ctx
            .panels
            .get(name)
            .cloned()
            .ok_or_else(|| EngineError::Eval(format!("unknown series '{name}'")))?,
        Const { value } => {
            // Const is only meaningful inside scalar ops, handled by callers.
            // Reject standalone evaluation.
            return Err(EngineError::Eval(format!("bare Const {value} not allowed at top level")));
        }
        Average { of, n } => eval(of, ctx)?.average(*n),
        Ema { of, n } => eval(of, ctx)?.ema(*n),
        Std { of, n } => eval(of, ctx)?.rolling_std(*n),
        Rsi { of, n } => eval(of, ctx)?.rsi(*n),
        PctChange { of, n } => eval(of, ctx)?.pct_change(*n),
        Rise { of, n } => eval(of, ctx)?.rise(*n),
        Shift { of, n } => eval(of, ctx)?.shift(*n),
        RollingMax { of, n } => eval(of, ctx)?.rolling_max(*n),
        Atr { high, low, close, n } => {
            ta::atr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        Natr { high, low, close, n } => {
            ta::natr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        WillR { high, low, close, n } => {
            ta::willr(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        Cci { high, low, close, n } => {
            ta::cci(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        StochK { high, low, close, n } => {
            ta::stoch_k(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        StochD { high, low, close, n, d } => {
            ta::stoch_d(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n, *d)
        }
        AroonUp { high, n } => ta::aroon_up(&eval(high, ctx)?, *n),
        AroonDown { low, n } => ta::aroon_down(&eval(low, ctx)?, *n),
        Adx { high, low, close, n } => {
            ta::adx(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        PlusDi { high, low, close, n } => {
            ta::plus_di(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        MinusDi { high, low, close, n } => {
            ta::minus_di(&eval(high, ctx)?, &eval(low, ctx)?, &eval(close, ctx)?, *n)
        }
        Obv { close, volume } => ta::obv(&eval(close, ctx)?, &eval(volume, ctx)?),
        Mfi { high, low, close, volume, n } => ta::mfi(
            &eval(high, ctx)?,
            &eval(low, ctx)?,
            &eval(close, ctx)?,
            &eval(volume, ctx)?,
            *n,
        ),
        Vwap { high, low, close, volume, n } => ta::vwap(
            &eval(high, ctx)?,
            &eval(low, ctx)?,
            &eval(close, ctx)?,
            &eval(volume, ctx)?,
            *n,
        ),
        Fall { of, n } => eval(of, ctx)?.fall(*n),
        IsLargest { of, n } => eval(of, ctx)?.is_largest(*n),
        IsSmallest { of, n } => eval(of, ctx)?.is_smallest(*n),
        Sustain { of, nwindow, nsatisfy } => eval(of, ctx)?.sustain(*nwindow, *nsatisfy),
        IsEntry { of } => eval(of, ctx)?.is_entry(),
        IsExit { of } => eval(of, ctx)?.is_exit(),
        Gt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x > y))?,
        Lt { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x < y))?,
        Ge { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x >= y))?,
        Le { l, r } => num_binop(l, r, ctx, |x, y| bool_to_f64(x <= y))?,
        And { l, r } => eval(l, ctx)?.and(&eval(r, ctx)?),
        Or { l, r } => eval(l, ctx)?.or(&eval(r, ctx)?),
        Add { l, r } => num_binop(l, r, ctx, |x, y| x + y)?,
        Sub { l, r } => num_binop(l, r, ctx, |x, y| x - y)?,
        Mul { l, r } => num_binop(l, r, ctx, |x, y| x * y)?,
        Div { l, r } => num_binop(l, r, ctx, |x, y| x / y)?,
        Neg { of } => eval(of, ctx)?.neg(),
        Ceil { of } => eval(of, ctx)?.ceil(),
        Rank { of, pct, ascending } => eval(of, ctx)?.rank_cs(*pct, *ascending),
        Mask { of, by } => eval(of, ctx)?.mask(&eval(by, ctx)?),
        HoldUntil {
            entry,
            exit,
            nstocks_limit,
            rank,
            stop_loss,
            take_profit,
            trail_stop,
            trail_stop_activation,
        } => {
            let e = eval(entry, ctx)?;
            let x = eval(exit, ctx)?;
            let rank_panel = match rank {
                Some(r) => Some(eval(r, ctx)?),
                None => None,
            };
            // Keep Default sentinels (±INF = off) for unset stops; override only what's set.
            let mut opts = HoldUntilOpts {
                nstocks_limit: *nstocks_limit,
                rank: rank_panel,
                ..Default::default()
            };
            if let Some(v) = stop_loss {
                opts.stop_loss = *v;
            }
            if let Some(v) = take_profit {
                opts.take_profit = *v;
            }
            if let Some(v) = trail_stop {
                opts.trail_stop = *v;
            }
            if let Some(v) = trail_stop_activation {
                opts.trail_stop_activation = *v;
            }
            // Stops price off close — supply it from the context when any stop is set.
            if stop_loss.is_some() || take_profit.is_some() || trail_stop.is_some() {
                opts.price = Some(ctx.panels.get("close").cloned().ok_or_else(|| {
                    EngineError::Eval("HoldUntil stops require a 'close' panel".into())
                })?);
            }
            e.hold_until(&x, &opts)
        }
        Rebalance { of, freq, on } => {
            let p = eval(of, ctx)?;
            match (freq.as_deref(), on) {
                (Some(_), Some(_)) => {
                    return Err(EngineError::Eval(
                        "Rebalance takes either freq or on, not both".into(),
                    ))
                }
                (None, None) => {
                    return Err(EngineError::Eval("Rebalance needs freq or on".into()))
                }
                (Some(freq), None) => {
                    let f = match freq {
                        "W" => Freq::Weekly,
                        "ME" => Freq::MonthEnd,
                        "QE" => Freq::QuarterEnd,
                        other => return Err(EngineError::Eval(format!("bad freq '{other}'"))),
                    };
                    p.rebalance_freq(f)
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
                    p.rebalance_dates(&dates)
                }
            }
        }
        Neutralize { of, by, add_const } => {
            let factor = eval(of, ctx)?;
            let neutralizers = by.iter().map(|b| eval(b, ctx)).collect::<Result<Vec<_>, _>>()?;
            factor.neutralize(&neutralizers, *add_const)
        }
        NeutralizeIndustry { of, add_const } => {
            eval(of, ctx)?.neutralize_industry(&ctx.industry, *add_const)
        }
        IndustryRank { of, categories } => {
            eval(of, ctx)?.industry_rank(&ctx.industry, categories.as_deref())
        }
        GroupbyCategory { of, agg } => eval(of, ctx)?.groupby_category(&ctx.industry, agg)?,
    })
}

use crate::backtest::{run, BacktestConfig};
use crate::report::{build_report, Report};

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
    let high = ctx.panels.get("high");
    let low = ctx.panels.get("low");
    Ok(build_report(run(&positions, prices, high, low, cfg)))
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
        let rank =
            run_strategy(&format!(r#"{{"op":"Rank","of":{d},"pct":true,"ascending":true}}"#), &c)
                .unwrap();
        assert_eq!(rank.data[[0, 0]], 0.5);
        assert_eq!(rank.data[[0, 1]], 1.0);
        // Add close + close, row0 [1,4] => [2,8]
        let add = run_strategy(&format!(r#"{{"op":"Add","l":{d},"r":{d}}}"#), &c).unwrap();
        assert_eq!(add.data[[0, 0]], 2.0);
        assert_eq!(add.data[[0, 1]], 8.0);
        // Ceil(close * 0.5), row1 [2,3] => [1.0,1.5] => [1,2]
        let ceil = run_strategy(
            &format!(r#"{{"op":"Ceil","of":{{"op":"Mul","l":{d},"r":{{"op":"Const","value":0.5}}}}}}"#),
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
        let r = run_strategy(r#"{"op":"Shift","of":{"op":"Data","name":"close"},"n":1}"#, &c).unwrap();
        assert!(r.data[[0, 0]].is_nan());
        assert_eq!(r.data[[1, 0]], 1.0);
        assert_eq!(r.data[[2, 0]], 2.0);
    }

    #[test]
    fn rolling_max_variant_evaluates() {
        let c = ctx();
        // ctx close col A = [1,2,3]; rolling_max(2) -> [NaN,2,3]
        let r = run_strategy(r#"{"op":"RollingMax","of":{"op":"Data","name":"close"},"n":2}"#, &c).unwrap();
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
            ("volume".into(), mk(100.0, 110.0, 120.0, 130.0, 140.0, 150.0)),
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

    #[test]
    fn hold_until_stops_default_price_to_close() {
        // ctx close col A = [1,2,3] (always > 0, so entry always true, exit never).
        // Without a stop, A is held every row. With take_profit=0.2 priced off the
        // close panel, A's return from entry (3/1 = 2.0 by row 2) blows past the
        // threshold and forces an exit — so the two runs MUST differ. This proves
        // the take_profit value is actually wired into HoldUntilOpts (not dropped
        // and left at the INFINITY sentinel, which would make the outputs identical).
        let c = ctx();
        let entry = r#"{ "op": "Gt", "l": { "op": "Data", "name": "close" }, "r": { "op": "Const", "value": 0.0 } }"#;
        let exit = r#"{ "op": "Lt", "l": { "op": "Data", "name": "close" }, "r": { "op": "Const", "value": 0.0 } }"#;
        let no_stop = format!(r#"{{ "op": "HoldUntil", "entry": {entry}, "exit": {exit} }}"#);
        let with_tp =
            format!(r#"{{ "op": "HoldUntil", "entry": {entry}, "exit": {exit}, "take_profit": 0.2 }}"#);

        let held = run_strategy(&no_stop, &c).unwrap();
        let stopped = run_strategy(&with_tp, &c).unwrap();

        // No-stop baseline: A held the whole way up the column.
        assert_eq!(held.data[[2, 0]], 1.0);
        // take_profit forces A out by row 2 (return 3/1 = 2.0 >> 0.2).
        assert_eq!(stopped.data[[2, 0]], 0.0);
    }

    #[test]
    fn hold_until_stops_error_without_close_panel() {
        // A context whose only panel is "signal" (no "close") must error when stops are set.
        let signal = Panel::new(
            vec![20240102, 20240103],
            vec!["A".into()],
            ndarray::array![[1.0], [1.0]],
        )
        .unwrap();
        let c = EvalContext::new(HashMap::from([("signal".to_string(), signal)]));
        let spec = r#"{
            "op": "HoldUntil",
            "entry": { "op": "Data", "name": "signal" },
            "exit": { "op": "Data", "name": "signal" },
            "stop_loss": 0.1
        }"#;
        assert!(run_strategy(spec, &c).is_err());
    }

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
            format!(r#"{{"op":"GroupbyCategory","of":{d},"agg":"mean"}}"#),
        ];
        for s in &specs {
            assert!(run_strategy(s, &ctx()).is_ok(), "failed to evaluate: {s}");
        }
    }
}
