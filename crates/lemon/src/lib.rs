//! Lemon — the strategy DSL for the citrus backtesting engine.
//!
//! Lemon is a small text language for writing trading strategies. A strategy
//! such as `close > sma(close, 2)` is *lowered* into a JSON `Expr` tree — the
//! serializable strategy AST ([`spec::Expr`]) — which the **yuzu** engine walks
//! against price/fundamental data to produce a position matrix. The same JSON
//! runs identically in-browser (WASM) and in the native batch runner.
//!
//! This crate does no math itself: it is a **surface syntax over the `Expr`
//! AST**. [`parse`] turns text into the JSON tree; [`format()`] renders a tree
//! back into canonical, re-indented text (a "gofmt for lemon"). All numeric
//! semantics live in the engine crate.
//!
//! # Layout
//!
//! - [`spec`] — the serializable [`Expr`] AST (JSON, tagged by `"op"`).
//! - [`services`] — editor language services (diagnostics, hover, completions),
//!   pure functions of `(source, position)` that back both the WASM boundary and
//!   the `lemon-lsp` language server.
//! - `dsl` (private) — the text syntax: `lex` (tokenizer), `parse` (text →
//!   JSON), `ops` (surface-name ⇄ op-tag vocabulary), `print` (JSON → text).
//!
//! # Example
//!
//! ```
//! let tree = lemon::parse("close > sma(close, 2)").unwrap();
//! assert_eq!(tree["op"], "Gt");
//! // `format` renders a tree back to canonical DSL text.
//! let text = lemon::format(&tree);
//! assert_eq!(text, "(close > sma(close, 2))");
//! ```
//!
//! A [`ParseError`] carries a 1-based `line`/`col` and prints as
//! `line:col: message`.
//!
//! # Syntax reference
//!
//! The author-facing language reference (lexical rules, operators and
//! precedence, the `let`/call grammar, and the complete op table) lives in
//! `docs/lemon.md` in the repository.

mod dsl;
pub mod services;
pub mod spec;

pub use dsl::{format, lint, parse, parse_analyzed, Analysis, Lint, ParseError};
pub use spec::Expr;

/// Machine-readable op vocabulary: the catalog of callable ops (canonical names,
/// aliases, tags, ordered field specs, defaults, descriptions) plus the binary
/// operators. This is the single source of truth the schema generator
/// (`cargo run -p lemon-lang --example gen-schema`) reads to emit
/// `schema/op-catalog.json` and `schema/lemon-spec.schema.json`.
pub mod meta {
    pub use crate::dsl::ops::{
        binary_operators, field_default, function_ops, FieldInfo, OpInfo, ALL_OP_TAGS,
    };
}
