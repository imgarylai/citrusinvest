//! `#!` front-matter: runner metadata at the top of a `.lemon` file.
//!
//! `#` already starts a line comment in lemon, so a `#!` line is invisible to
//! `lemon::parse` — the language grammar is untouched and the spec tree stays
//! pure. The **runner** (and `lemon check`) reads these lines instead:
//!
//! ```text
//! #! name: Momentum v2
//! #! universe: 20180101..20241231
//! #! symbols: AAPL, MSFT, NVDA
//! #! config: { "fee_ratio": 0.001, "stop_loss": 0.08 }
//! #! price-key: close
//! ```
//!
//! Rules (fail-loudly, like the rest of the language):
//! - The front-matter is the leading run of `#!` lines (blank lines allowed
//!   between them). A `#!` line after the strategy source has started is an
//!   **error**, not a comment — a silently ignored directive would change the
//!   run parameters behind the author's back.
//! - Unknown keys, duplicate keys, and malformed values are line-labelled errors.
//! - `config` is a JSON object using the same flat knob names as the
//!   `yuzu-server` request / `yuzu-cli` flags (`fee_ratio`, `slippage_ratio`,
//!   `stop_loss`, `stop_fill`, …). Unknown knobs are rejected (typos like
//!   `fee_ration` must not silently become a zero-fee run).
//! - `universe` is a `FROM..TO` date window (`YYYYMMDD..YYYYMMDD`); either side
//!   may be omitted (`20180101..`).
//! - `symbols` is a comma-separated explicit universe — cross-sectional ops
//!   then see exactly these names. Beware: a list frozen *today* implies
//!   survivorship bias in a historical run; point-in-time index universes
//!   (`symbols_hint`) are issue #245.
//!
//! CLI flags override front-matter; front-matter overrides built-in defaults.

use serde::Deserialize;
use yuzu_core::backtest::{BacktestConfig, StopConfig, StopFill};

/// A line-labelled front-matter error. `line` is 1-based.
#[derive(Debug)]
pub struct FmError {
    pub line: usize,
    pub message: String,
}

impl FmError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        FmError {
            line,
            message: message.into(),
        }
    }
}

/// The engine config knobs accepted in a `config` object (front-matter or
/// envelope). Field names follow the `yuzu-server` request wire format — the
/// flat spelling of [`BacktestConfig`] plus the optional stop levels.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigDoc {
    pub fee_ratio: f64,
    pub tax_ratio: f64,
    pub position_limit: f64,
    pub slippage_ratio: f64,
    pub initial_capital: f64,
    pub max_participation: f64,
    pub impact_coef: f64,
    pub delist_after: usize,
    pub delist_haircut: f64,
    /// Benchmark symbol (its closes are loaded), e.g. `"SPY"`.
    pub benchmark: Option<String>,
    pub bootstrap_samples: usize,
    pub bootstrap_block: usize,
    pub live_performance_start: Option<i32>,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub trail_stop: Option<f64>,
    pub trail_stop_activation: f64,
    pub stop_fill: StopFillDoc,
}

/// Wire form of [`StopFill`]: `"touched"` (default) or `"close"`.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopFillDoc {
    #[default]
    Touched,
    Close,
}

impl ConfigDoc {
    pub fn to_backtest_config(&self) -> BacktestConfig {
        BacktestConfig {
            fee_ratio: self.fee_ratio,
            tax_ratio: self.tax_ratio,
            position_limit: self.position_limit,
            slippage_ratio: self.slippage_ratio,
            initial_capital: self.initial_capital,
            max_participation: self.max_participation,
            impact_coef: self.impact_coef,
            delist_after: self.delist_after,
            delist_haircut: self.delist_haircut,
            benchmark_key: self.benchmark.clone(),
            bootstrap_samples: self.bootstrap_samples,
            bootstrap_block: self.bootstrap_block,
            live_performance_start: self.live_performance_start,
            stops: StopConfig::from_options(
                self.stop_loss,
                self.take_profit,
                self.trail_stop,
                self.trail_stop_activation,
                match self.stop_fill {
                    StopFillDoc::Touched => StopFill::Touched,
                    StopFillDoc::Close => StopFill::Close,
                },
            ),
        }
    }
}

/// Parsed front-matter. Every field is optional — an absent key means "use the
/// flag or the default".
#[derive(Debug, Default)]
pub struct FrontMatter {
    pub name: Option<String>,
    pub from: Option<i32>,
    pub to: Option<i32>,
    pub symbols: Option<Vec<String>>,
    pub config: Option<ConfigDoc>,
    pub price_key: Option<String>,
}

/// True when a (whitespace-trimmed) line is a `#!` directive.
fn is_directive(line: &str) -> bool {
    line.trim_start().starts_with("#!")
}

/// Parse the front-matter block of a lemon source. Returns the (possibly
/// empty) metadata, or the first line-labelled error.
pub fn parse(src: &str) -> Result<FrontMatter, FmError> {
    let mut fm = FrontMatter::default();
    let mut seen: Vec<&str> = Vec::new();
    let mut in_block = true;

    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        if !is_directive(line) {
            // Blank lines and plain `#` comments may surround directives; the
            // first line of actual source ends the block.
            if !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                in_block = false;
            }
            continue;
        }
        if !in_block {
            return Err(FmError::new(
                lineno,
                "front-matter directive after the strategy source — move it to the top of the file",
            ));
        }
        let body = line.trim_start()[2..].trim();
        if body.is_empty() {
            return Err(FmError::new(lineno, "empty front-matter directive"));
        }
        let Some((key, value)) = body.split_once(':') else {
            return Err(FmError::new(
                lineno,
                "malformed front-matter directive: expected `#! key: value`",
            ));
        };
        let (key, value) = (key.trim(), value.trim());
        if value.is_empty() {
            return Err(FmError::new(lineno, format!("`{key}` needs a value")));
        }
        if seen.contains(&key) {
            return Err(FmError::new(
                lineno,
                format!("duplicate front-matter key `{key}`"),
            ));
        }
        match key {
            "name" => fm.name = Some(value.to_string()),
            "universe" => (fm.from, fm.to) = parse_universe(lineno, value)?,
            "symbols" => {
                fm.symbols = Some(parse_symbol_list(value).map_err(|m| FmError::new(lineno, m))?)
            }
            "config" => fm.config = Some(parse_config(lineno, value)?),
            "price-key" => fm.price_key = Some(parse_price_key(lineno, value)?),
            _ => {
                return Err(FmError::new(
                    lineno,
                    format!(
                        "unknown front-matter key `{key}` (expected `name`, `universe`, `symbols`, `config`, or `price-key`)"
                    ),
                ));
            }
        }
        seen.push(key);
    }
    Ok(fm)
}

/// `FROM..TO` — either side may be empty (open-ended).
fn parse_universe(line: usize, value: &str) -> Result<(Option<i32>, Option<i32>), FmError> {
    let err = || {
        FmError::new(
            line,
            format!(
                "`universe` expects a `FROM..TO` date window (YYYYMMDD..YYYYMMDD, either side optional), got `{value}`"
            ),
        )
    };
    let (lo, hi) = value.split_once("..").ok_or_else(err)?;
    let parse_side = |s: &str| -> Result<Option<i32>, FmError> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        s.parse::<i32>().map(Some).map_err(|_| err())
    };
    let (from, to) = (parse_side(lo)?, parse_side(hi)?);
    if from.is_none() && to.is_none() {
        return Err(err());
    }
    Ok((from, to))
}

fn parse_config(line: usize, value: &str) -> Result<ConfigDoc, FmError> {
    let err = |e: String| FmError::new(line, format!("invalid `config` object: {e}"));
    let v: serde_json::Value = serde_json::from_str(value).map_err(|e| err(e.to_string()))?;
    // Guard before deserializing: serde would fill a struct positionally from a
    // JSON array (`[1, 2]` → fee_ratio=1, tax_ratio=2), which is exactly the
    // silent misconfiguration this format refuses.
    if !v.is_object() {
        return Err(err("expected a JSON object".into()));
    }
    serde_json::from_value::<ConfigDoc>(v).map_err(|e| err(e.to_string()))
}

/// Parse a comma-separated symbol list (`AAPL, MSFT`). Shared with the
/// `--symbols` flag, so the error text names the problem, not the source.
pub fn parse_symbol_list(value: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for part in value.split(',') {
        let sym = part.trim();
        if sym.is_empty() {
            return Err(format!("empty symbol in list `{value}`"));
        }
        if out.iter().any(|s| s == sym) {
            return Err(format!("duplicate symbol `{sym}`"));
        }
        out.push(sym.to_string());
    }
    Ok(out)
}

fn parse_price_key(line: usize, value: &str) -> Result<String, FmError> {
    match value {
        "open" | "high" | "low" | "close" => Ok(value.to_string()),
        _ => Err(FmError::new(
            line,
            format!("`price-key` must be one of open/high/low/close, got `{value}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_key_and_leaves_the_source_alone() {
        let src = "#! name: Momentum v2\n\n#! universe: 20180101..20241231\n#! config: { \"fee_ratio\": 0.001, \"stop_loss\": 0.08, \"stop_fill\": \"close\" }\n#! price-key: open\n\nclose > sma(close, 20)\n";
        let fm = parse(src).unwrap();
        assert_eq!(fm.name.as_deref(), Some("Momentum v2"));
        assert_eq!((fm.from, fm.to), (Some(20180101), Some(20241231)));
        assert_eq!(fm.price_key.as_deref(), Some("open"));
        let cfg = fm.config.unwrap().to_backtest_config();
        assert_eq!(cfg.fee_ratio, 0.001);
        assert_eq!(cfg.stops.stop_loss, 0.08);
        assert!(matches!(cfg.stops.fill, StopFill::Close));
        // The source itself still parses — directives are comments to the lexer.
        assert!(lemon::parse(src).is_ok());
    }

    #[test]
    fn no_front_matter_is_fine() {
        let fm = parse("close > sma(close, 2)").unwrap();
        assert!(fm.name.is_none() && fm.from.is_none() && fm.config.is_none());
    }

    #[test]
    fn universe_sides_are_optional_but_not_both() {
        let fm = parse("#! universe: 20180101..\nclose > 1").unwrap();
        assert_eq!((fm.from, fm.to), (Some(20180101), None));
        let fm = parse("#! universe: ..20241231\nclose > 1").unwrap();
        assert_eq!((fm.from, fm.to), (None, Some(20241231)));
        let err = parse("#! universe: ..\nclose > 1").unwrap_err();
        assert!(err.message.contains("FROM..TO"), "{}", err.message);
        let err = parse("#! universe: 2018-01-01..2024\nclose > 1").unwrap_err();
        assert_eq!(err.line, 1);
        let err = parse("#! universe: sp500\nclose > 1").unwrap_err();
        assert!(err.message.contains("FROM..TO"), "{}", err.message);
    }

    #[test]
    fn config_typos_fail_loudly() {
        // `fee_ration` must not silently become a zero-fee run.
        let err = parse("#! config: { \"fee_ration\": 0.001 }\nclose > 1").unwrap_err();
        assert!(err.message.contains("fee_ration"), "{}", err.message);
        let err = parse("#! config: [1, 2]\nclose > 1").unwrap_err();
        assert!(err.message.contains("invalid `config`"), "{}", err.message);
        let err = parse("#! config: not-json\nclose > 1").unwrap_err();
        assert!(err.message.contains("invalid `config`"), "{}", err.message);
    }

    #[test]
    fn full_knob_set_maps_onto_backtest_config() {
        let src = r#"#! config: { "fee_ratio": 0.001, "tax_ratio": 0.003, "position_limit": 0.1, "slippage_ratio": 0.0005, "initial_capital": 1000000, "max_participation": 0.05, "impact_coef": 0.1, "delist_after": 10, "delist_haircut": 0.5, "benchmark": "SPY", "bootstrap_samples": 100, "bootstrap_block": 5, "live_performance_start": 20240101, "take_profit": 0.2, "trail_stop": 0.1, "trail_stop_activation": 0.05 }
close > 1"#;
        let cfg = parse(src).unwrap().config.unwrap().to_backtest_config();
        assert_eq!(cfg.tax_ratio, 0.003);
        assert_eq!(cfg.position_limit, 0.1);
        assert_eq!(cfg.initial_capital, 1_000_000.0);
        assert_eq!(cfg.max_participation, 0.05);
        assert_eq!(cfg.impact_coef, 0.1);
        assert_eq!(cfg.delist_after, 10);
        assert_eq!(cfg.delist_haircut, 0.5);
        assert_eq!(cfg.benchmark_key.as_deref(), Some("SPY"));
        assert_eq!(cfg.bootstrap_samples, 100);
        assert_eq!(cfg.bootstrap_block, 5);
        assert_eq!(cfg.live_performance_start, Some(20240101));
        assert_eq!(cfg.stops.take_profit, 0.2);
        assert_eq!(cfg.stops.trail_stop, 0.1);
        assert_eq!(cfg.stops.trail_stop_activation, 0.05);
        // stop_loss stays off (−INF sentinel) when unset.
        assert_eq!(cfg.stops.stop_loss, f64::NEG_INFINITY);
        assert!(matches!(cfg.stops.fill, StopFill::Touched));
    }

    #[test]
    fn rejects_unknown_duplicate_and_malformed_directives() {
        let err = parse("#! nmae: X\nclose > 1").unwrap_err();
        assert!(err.message.contains("unknown front-matter key `nmae`"));
        let err = parse("#! name: A\n#! name: B\nclose > 1").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.message.contains("duplicate"));
        let err = parse("#! just words\nclose > 1").unwrap_err();
        assert!(err.message.contains("expected `#! key: value`"));
        let err = parse("#!\nclose > 1").unwrap_err();
        assert!(err.message.contains("empty front-matter directive"));
        let err = parse("#! name:\nclose > 1").unwrap_err();
        assert!(err.message.contains("`name` needs a value"));
        let err = parse("#! price-key: nav\nclose > 1").unwrap_err();
        assert!(err.message.contains("open/high/low/close"));
    }

    #[test]
    fn directive_after_source_is_an_error_not_a_comment() {
        let err = parse("#! name: X\nclose > 1\n#! universe: 20180101..\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(
            err.message.contains("move it to the top"),
            "{}",
            err.message
        );
        // Indented directives count too — they must not silently degrade to comments.
        let err = parse("close > 1\n  #! name: X\n").unwrap_err();
        assert_eq!(err.line, 2);
    }

    #[test]
    fn symbols_parse_trimmed_and_reject_junk() {
        let fm = parse("#! symbols: AAPL, MSFT,NVDA\nclose > 1").unwrap();
        assert_eq!(
            fm.symbols.as_deref(),
            Some(&["AAPL".to_string(), "MSFT".into(), "NVDA".into()][..])
        );
        let err = parse("#! symbols: AAPL,,MSFT\nclose > 1").unwrap_err();
        assert!(err.message.contains("empty symbol"), "{}", err.message);
        let err = parse("#! symbols: AAPL, AAPL\nclose > 1").unwrap_err();
        assert!(err.message.contains("duplicate symbol"), "{}", err.message);
    }

    #[test]
    fn plain_comments_and_blanks_may_surround_the_block() {
        let src =
            "# strategy file\n\n#! name: X\n# midway note\n#! universe: 20180101..\n\nclose > 1\n";
        let fm = parse(src).unwrap();
        assert_eq!(fm.name.as_deref(), Some("X"));
        assert_eq!(fm.from, Some(20180101));
    }
}
