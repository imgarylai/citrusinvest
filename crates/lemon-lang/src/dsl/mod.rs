//! Human-writable DSL ⇄ JSON `Expr` tree. `parse` lowers DSL text to the same
//! `serde_json::Value` the engine deserializes into [`crate::spec::Expr`];
//! `print` renders a tree back to DSL text (flat — no `let` reconstruction).
//!
//! Sub-modules:
//! - [`lex`] — tokenizer: numbers (with `_`/`e`), strings (no escapes), `#`
//!   comments, identifiers, operators, `let`.
//! - [`parse`] — recursive-descent + Pratt parser; handles `let` inlining,
//!   positional-then-keyword call args, and `Const` promotion of bare numbers.
//! - [`ops`] — the op vocabulary: surface names ⇄ `Expr` op tags and field
//!   layout (the single source both `parse` and `print` consult).
//! - [`print`] — the canonical formatter: JSON `Expr` → indented DSL text.

pub(crate) mod lex;
mod lint;
pub mod ops;
mod parse;
mod print;

/// A parse failure with a 1-based source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

pub use lint::{lint, Lint};
pub use parse::{parse, parse_analyzed, Analysis};
pub use print::format;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_displays_with_position() {
        let e = ParseError {
            line: 3,
            col: 7,
            message: "bad token".into(),
        };
        assert_eq!(e.to_string(), "3:7: bad token");
    }
}

#[cfg(test)]
mod roundtrip {
    use super::{format, parse};
    use crate::spec::Expr;
    use serde_json::{json, Value};

    fn corpus() -> Vec<Value> {
        vec![
            json!({"op":"Gt","l":{"op":"Data","name":"close"},
                   "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":2}}),
            json!({"op":"Rank","of":{"op":"Neg","of":{"op":"Data","name":"pe"}}}),
            json!({"op":"Vwap","high":{"op":"Data","name":"high"},"low":{"op":"Data","name":"low"},
                   "close":{"op":"Data","name":"close"},"volume":{"op":"Data","name":"volume"},"n":20}),
            json!({"op":"Mask","of":{"op":"IsLargest","of":{"op":"Data","name":"x"},"n":30},
                   "by":{"op":"Gt","l":{"op":"Data","name":"market_cap"},"r":{"op":"Const","value":500000000}}}),
            json!({"op":"Sustain","of":{"op":"Data","name":"x"},"nwindow":3,"nsatisfy":2}),
            json!({"op":"Rebalance","of":{"op":"Data","name":"x"},"freq":"ME"}),
            json!({"op":"Neutralize","of":{"op":"Data","name":"x"},
                   "by":[{"op":"Data","name":"pe"},{"op":"Data","name":"market_cap"}]}),
            json!({"op":"NeutralizeIndustry","of":{"op":"Data","name":"x"}}),
            json!({"op":"HoldUntil",
                   "entry":{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":1}},
                   "exit":{"op":"Lt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":1}},
                   "nstocks_limit":1}),
            json!({"op":"Add",
                   "l":{"op":"Mul","l":{"op":"Const","value":2},"r":{"op":"Data","name":"x"}},
                   "r":{"op":"Data","name":"y"}}),
            json!({"op":"IndustryRank","of":{"op":"Data","name":"x"},
                "categories":["tech","fin"]}),
        ]
    }

    #[test]
    fn corpus_is_valid_expr() {
        for tree in corpus() {
            let r: Result<Expr, _> = serde_json::from_value(tree.clone());
            assert!(r.is_ok(), "not a valid Expr: {tree}");
        }
    }

    #[test]
    fn parse_of_print_is_identity() {
        for tree in corpus() {
            let text = format(&tree);
            let back = parse(&text).unwrap_or_else(|e| panic!("reparse failed for `{text}`: {e}"));
            assert_eq!(back, tree, "round-trip mismatch via `{text}`");
        }
    }

    /// Formatting is a published contract: re-formatting already-formatted source
    /// must be a no-op, or every consumer's diffs churn. Guards against a future
    /// printer change that is stable on raw trees but not on its own output.
    #[test]
    fn print_is_idempotent() {
        for tree in corpus() {
            let once = format(&tree);
            let reparsed = parse(&once).unwrap();
            assert_eq!(
                format(&reparsed),
                once,
                "format not idempotent for `{once}`"
            );
        }
    }
}
