//! [`EngineError`]: shape violations from panel construction and evaluation failures.

use thiserror::Error;

/// Errors from panel construction, strategy evaluation, and backtest setup.
///
/// Callers (CLI / server / WASM) typically surface these via [`Display`] /
/// [`ToString`]; match on variants when you need structured handling.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("shape mismatch: {rows}x{cols} != data len {data_len}")]
    ShapeMismatch {
        rows: usize,
        cols: usize,
        data_len: usize,
    },

    #[error("unknown series '{name}'")]
    UnknownSeries { name: String },

    #[error("unknown price series '{key}'")]
    UnknownPriceKey { key: String },

    #[error("unknown benchmark series '{key}'")]
    UnknownBenchmark { key: String },

    #[error("benchmark series '{key}' has no symbols")]
    EmptyBenchmark { key: String },

    #[error("bare Const {value} not allowed at top level")]
    BareConst { value: f64 },

    #[error("bad freq '{freq}'")]
    BadFreq { freq: String },

    #[error("Rebalance takes either freq or on, not both")]
    RebalanceBoth,

    #[error("Rebalance needs freq or on")]
    RebalanceNeither,

    #[error("bad groupby agg '{agg}'")]
    BadGroupbyAgg { agg: String },

    #[error("both operands of a binary op are Const")]
    BothOperandsConst,

    #[error("spec parse error: {0}")]
    SpecParse(String),

    /// Catch-all for residual evaluation failures.
    #[error("eval error: {0}")]
    Eval(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        let e = EngineError::ShapeMismatch {
            rows: 1,
            cols: 2,
            data_len: 3,
        };
        assert!(e.to_string().contains("shape mismatch"));
        assert_eq!(
            EngineError::UnknownSeries { name: "pe".into() }.to_string(),
            "unknown series 'pe'"
        );
        assert_eq!(
            EngineError::BadFreq { freq: "X".into() }.to_string(),
            "bad freq 'X'"
        );
        assert!(EngineError::Eval("boom".into())
            .to_string()
            .contains("boom"));
        assert!(EngineError::SpecParse("eof".into())
            .to_string()
            .contains("spec parse"));
    }

    #[test]
    fn is_std_error() {
        let e: Box<dyn std::error::Error> = Box::new(EngineError::RebalanceNeither);
        assert!(e.to_string().contains("Rebalance"));
    }
}
