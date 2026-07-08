//! [`EngineError`]: shape violations from panel construction and evaluation failures.

use std::fmt;

#[derive(Debug)]
pub enum EngineError {
    ShapeMismatch { rows: usize, cols: usize, data_len: usize },
    Eval(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::ShapeMismatch { rows, cols, data_len } => {
                write!(f, "shape mismatch: {rows}x{cols} != data len {data_len}")
            }
            EngineError::Eval(m) => write!(f, "eval error: {m}"),
        }
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        let e = EngineError::ShapeMismatch { rows: 1, cols: 2, data_len: 3 };
        assert!(e.to_string().contains("shape mismatch"));
        assert!(EngineError::Eval("boom".into()).to_string().contains("boom"));
    }
}
