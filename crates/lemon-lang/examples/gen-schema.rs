//! Regenerate the machine-readable references for the lemon DSL.
//!
//! Regeneration command (run from anywhere in the workspace):
//!
//!     cargo run -p lemon-lang --example gen-schema
//!
//! Writes two files into the workspace-root `schema/` directory:
//!
//!   - `schema/op-catalog.json`       — the full callable op vocabulary as JSON
//!                                       (canonical names, aliases, ordered args,
//!                                       kinds, defaults, one-line descriptions).
//!   - `schema/lemon-spec.schema.json` — a JSON Schema (draft 2020-12) for the
//!                                       tagged-by-`"op"` `Expr` tree, so an LLM
//!                                       using structured output can be constrained
//!                                       to emit only valid specs, and a spec from
//!                                       `lemon::parse` can be validated.
//!
//! Both artifacts are DERIVED from `lemon::meta` (the `ROWS`/`Expr` metadata in
//! `crates/lemon/src/dsl/ops.rs` + `spec.rs`) — the single source of truth — so
//! they can never drift from what the parser accepts. No heavy dependency: the
//! JSON is hand-emitted from that metadata with `serde_json` (already a dep).
//! If you add or change an op, edit `ops.rs`/`spec.rs` and rerun this example.

use std::path::PathBuf;

use lemon::meta;
use serde_json::{json, Map, Value};

/// JSON schema fragment for a single field, by its metadata `kind`.
fn field_schema(kind: &str) -> Value {
    match kind {
        "expr" => json!({ "$ref": "#/$defs/Expr" }),
        "number" => json!({ "type": "number" }),
        "bool" => json!({ "type": "boolean" }),
        "string" => json!({ "type": "string" }),
        // A list of expressions.
        "list" => json!({ "type": "array", "items": { "$ref": "#/$defs/Expr" } }),
        "list-string" => json!({ "type": "array", "items": { "type": "string" } }),
        other => panic!("unknown field kind `{other}`"),
    }
}

/// Build the `op-catalog.json` value: the complete callable vocabulary.
fn build_catalog() -> Value {
    let mut ops: Vec<Value> = Vec::new();

    // Two leaves.
    ops.push(json!({
        "name": "Data",
        "tag": "Data",
        "form": "leaf",
        "description": "A raw input series by name (e.g. close, pe, revenue_growth). In DSL, a bare identifier lowers to this.",
        "args": [
            { "name": "name", "kind": "string", "required": true }
        ]
    }));
    ops.push(json!({
        "name": "Const",
        "tag": "Const",
        "form": "leaf",
        "description": "A constant scalar value, broadcast across the panel. In DSL, a bare number operand lowers to this.",
        "args": [
            { "name": "value", "kind": "number", "required": true }
        ]
    }));

    // Function-style ops.
    for op in meta::function_ops() {
        let args: Vec<Value> = op
            .fields
            .iter()
            .map(|f| {
                let mut m = Map::new();
                m.insert("name".into(), json!(f.name));
                m.insert("kind".into(), json!(f.kind));
                m.insert("required".into(), json!(f.required));
                if let Some(d) = &f.default {
                    m.insert("default".into(), d.clone());
                }
                Value::Object(m)
            })
            .collect();
        ops.push(json!({
            "name": op.name,
            "aliases": op.aliases,
            "tag": op.tag,
            "form": "function",
            "description": op.description,
            "args": args,
        }));
    }

    // Binary operators (l, r).
    for (symbol, tag) in meta::binary_operators() {
        ops.push(json!({
            "name": symbol,
            "tag": tag,
            "form": "operator",
            "description": binop_description(tag),
            "args": [
                { "name": "l", "kind": "expr", "required": true },
                { "name": "r", "kind": "expr", "required": true }
            ]
        }));
    }

    // Unary operators: negation and logical NOT.
    ops.push(json!({
        "name": "-",
        "tag": "Neg",
        "form": "operator",
        "unary": true,
        "description": "Negation (-`of`).",
        "args": [
            { "name": "of", "kind": "expr", "required": true }
        ]
    }));
    ops.push(json!({
        "name": "not",
        "tag": "Not",
        "form": "operator",
        "unary": true,
        "description": "Logical NOT of a boolean panel (NaN is falsy, so `not` of NaN is 1).",
        "args": [
            { "name": "of", "kind": "expr", "required": true }
        ]
    }));

    Value::Array(ops)
}

/// Terse descriptions for the operator ops (which live in `BINOPS`, not `ROWS`).
fn binop_description(tag: &str) -> &'static str {
    match tag {
        "Gt" => "1 where `l` is greater than `r`, else 0.",
        "Lt" => "1 where `l` is less than `r`, else 0.",
        "Ge" => "1 where `l` is greater than or equal to `r`, else 0.",
        "Le" => "1 where `l` is less than or equal to `r`, else 0.",
        "And" => "Logical AND of two boolean panels.",
        "Or" => "Logical OR of two boolean panels.",
        "Add" => "Element-wise `l` + `r`.",
        "Sub" => "Element-wise `l` - `r`.",
        "Mul" => "Element-wise `l` * `r`.",
        "Div" => "Element-wise `l` / `r`.",
        other => panic!("no description for operator tag `{other}`"),
    }
}

/// One `oneOf` branch of the Expr schema: an object tagged by `op` with its fields.
fn op_object_schema(
    tag: &str,
    description: &str,
    fields: &[(&str, &str, bool)], // (name, kind, required)
) -> Value {
    let mut properties = Map::new();
    properties.insert("op".into(), json!({ "const": tag }));
    let mut required: Vec<Value> = vec![json!("op")];
    for (name, kind, req) in fields {
        properties.insert((*name).into(), field_schema(kind));
        if *req {
            required.push(json!(*name));
        }
    }
    json!({
        "type": "object",
        "title": tag,
        "description": description,
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": false,
    })
}

/// Build the `lemon-spec.schema.json` value: a JSON Schema over the Expr tree.
fn build_spec_schema() -> Value {
    let mut branches: Vec<Value> = Vec::new();

    // Leaves.
    branches.push(op_object_schema(
        "Data",
        "A raw input series by name.",
        &[("name", "string", true)],
    ));
    branches.push(op_object_schema(
        "Const",
        "A constant scalar value, broadcast across the panel.",
        &[("value", "number", true)],
    ));

    // Function-style ops.
    for op in meta::function_ops() {
        let fields: Vec<(&str, &str, bool)> = op
            .fields
            .iter()
            .map(|f| (f.name, f.kind, f.required))
            .collect();
        branches.push(op_object_schema(op.tag, op.description, &fields));
    }

    // Binary operators.
    for (_, tag) in meta::binary_operators() {
        branches.push(op_object_schema(
            tag,
            binop_description(tag),
            &[("l", "expr", true), ("r", "expr", true)],
        ));
    }

    // Unary operators.
    branches.push(op_object_schema(
        "Neg",
        "Negation (-`of`).",
        &[("of", "expr", true)],
    ));
    branches.push(op_object_schema(
        "Not",
        "Logical NOT of a boolean panel (NaN is falsy, so `not` of NaN is 1).",
        &[("of", "expr", true)],
    ));

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/citrusquant/citrusquant/schema/lemon-spec.schema.json",
        "title": "Lemon strategy spec (Expr tree)",
        "description": "The serializable lemon strategy AST: a JSON tree tagged by \"op\". Generated from crates/lemon metadata; do not edit by hand.",
        "$ref": "#/$defs/Expr",
        "$defs": {
            "Expr": {
                "oneOf": branches
            }
        }
    })
}

/// Locate the workspace-root `schema/` directory (two levels up from this crate).
fn schema_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <workspace>/crates/lemon
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent() // crates
        .and_then(|p| p.parent()) // workspace root
        .expect("crate is two levels below workspace root")
        .join("schema")
}

fn write_pretty(path: &std::path::Path, value: &Value) {
    let mut s = serde_json::to_string_pretty(value).expect("serialize");
    s.push('\n');
    std::fs::write(path, s).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("wrote {}", path.display());
}

fn main() {
    let dir = schema_dir();
    std::fs::create_dir_all(&dir).expect("create schema dir");

    write_pretty(&dir.join("op-catalog.json"), &build_catalog());
    write_pretty(&dir.join("lemon-spec.schema.json"), &build_spec_schema());

    // Self-check: every op tag in ALL_OP_TAGS must appear as a schema branch, so
    // the generated schema covers the complete vocabulary the parser accepts.
    let schema = build_spec_schema();
    let branches = schema["$defs"]["Expr"]["oneOf"].as_array().unwrap();
    let tags: std::collections::HashSet<&str> = branches
        .iter()
        .map(|b| b["properties"]["op"]["const"].as_str().unwrap())
        .collect();
    for tag in meta::ALL_OP_TAGS {
        assert!(tags.contains(tag), "schema missing op tag `{tag}`");
    }
    assert_eq!(
        tags.len(),
        meta::ALL_OP_TAGS.len(),
        "schema branch count != ALL_OP_TAGS count"
    );
    println!("ok: {} op tags covered", tags.len());
}
