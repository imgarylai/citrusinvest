//! Lemon — the strategy DSL. `spec` is the serializable `Expr` AST; `dsl` is its
//! human-writable text syntax. `parse` lowers `.lemon` source to the JSON `Expr`
//! tree the engine evaluates; `format` renders a tree back to source.

pub mod spec;
mod dsl;

pub use dsl::{parse, format, ParseError};
pub use spec::Expr;
