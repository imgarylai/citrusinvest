//! The **shareable strategy envelope**: a small, versioned document that wraps a
//! strategy (as a lemon `source` string or a lowered `spec` [`Expr`] tree)
//! together with the metadata needed to reproduce a run — a name, an optional
//! description/author, the engine config knobs, a universe window, and the
//! engine version it was authored against.
//!
//! A strategy is pure data (no market data, ever — see the licensing note in
//! issue #30), so an envelope is safe to store, share, and re-run in another
//! browser. The envelope references series **names + universe**, never embedded
//! prices or fundamentals.
//!
//! # Shape
//!
//! ```jsonc
//! {
//!   "format": 1,                         // envelope version (this build: 1)
//!   "name": "Cheap quality rotation",
//!   "description": "…",                  // optional
//!   "author": "…",                       // optional
//!   "source": "rank(pe) < 20",           // lemon text — OR —
//!   "spec": { "op": "…", … },            // a lowered Expr tree (exactly one of the two)
//!   "config": { "fee_ratio": 0.001 },    // optional engine config (opaque here)
//!   "universe": { "from": 20180101, "to": 20251231, "symbols_hint": "sp500" },
//!   "engine_version": "yuzu-core 0.x"    // optional reproducibility pin
//! }
//! ```
//!
//! [`check`] validates the whole document and returns the resolved `Expr` tree
//! (parsed from `source`, or the `spec` as given), so a registry / web app can
//! reject malformed submissions with actionable errors and a runner can pull the
//! spec straight out. The JSON Schema for external consumers lives at
//! `schema/strategy-envelope.schema.json`.

use serde::Deserialize;
use serde_json::Value;

use crate::spec::Expr;

/// Envelope format version understood by this build. Bumped only on a
/// breaking change to the envelope shape (never for a new engine op).
pub const FORMAT_VERSION: u32 = 1;

/// The parsed envelope document. Field presence mirrors the JSON; semantic
/// validation (version, exactly-one-of spec/source, a well-formed spec) is done
/// by [`check`], not by deserialization alone.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// Envelope format version. Must equal [`FORMAT_VERSION`].
    pub format: u32,
    /// Human-readable strategy name (must be non-empty).
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// A lowered `Expr` tree. Exactly one of `spec` / `source` must be present.
    #[serde(default)]
    pub spec: Option<Value>,
    /// Lemon source text. Exactly one of `spec` / `source` must be present.
    #[serde(default)]
    pub source: Option<String>,
    /// Engine config knobs (fees, delist, etc.). Opaque at the language layer —
    /// the engine/runner interprets it; here we only require it be an object.
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default)]
    pub universe: Option<Universe>,
    /// Free-form engine-version pin for reproducibility (e.g. `"yuzu-core 0.4"`).
    #[serde(default)]
    pub engine_version: Option<String>,
}

/// The date window + universe hint a run should cover. Names/windows only — no
/// embedded data.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Universe {
    /// Inclusive start date, `YYYYMMDD`.
    #[serde(default)]
    pub from: Option<i64>,
    /// Inclusive end date, `YYYYMMDD`.
    #[serde(default)]
    pub to: Option<i64>,
    /// A named universe hint (e.g. `"sp500"`) the runner resolves to symbols.
    #[serde(default)]
    pub symbols_hint: Option<String>,
    /// An explicit symbol list, if the author pinned one.
    #[serde(default)]
    pub symbols: Option<Vec<String>>,
}

/// A validated envelope: the strategy name and the resolved `Expr` tree (parsed
/// from `source`, or the `spec` verbatim). Enough to hand to a runner.
#[derive(Debug)]
pub struct Checked {
    pub name: String,
    /// The lowered strategy tree, guaranteed to deserialize into [`Expr`].
    pub spec: Value,
}

/// Validate a strategy-envelope JSON document.
///
/// Returns [`Checked`] (name + resolved spec tree) on success, or the list of
/// human-readable problems found. Checks, in order: the JSON parses; the
/// envelope shape is valid (unknown keys rejected); `format` matches
/// [`FORMAT_VERSION`]; `name` is non-empty; exactly one of `spec`/`source` is
/// present; `source` parses as lemon / `spec` deserializes into a valid `Expr`;
/// `config` (if present) is an object.
pub fn check(doc: &str) -> Result<Checked, Vec<String>> {
    let value: Value = serde_json::from_str(doc).map_err(|e| vec![format!("invalid JSON: {e}")])?;
    check_value(&value)
}

/// Like [`check`], starting from an already-parsed JSON value.
pub fn check_value(value: &Value) -> Result<Checked, Vec<String>> {
    let env: Envelope = serde_json::from_value(value.clone())
        .map_err(|e| vec![format!("not a valid strategy envelope: {e}")])?;

    let mut errors: Vec<String> = Vec::new();

    if env.format != FORMAT_VERSION {
        errors.push(format!(
            "unsupported envelope format {} (this build understands format {FORMAT_VERSION})",
            env.format
        ));
    }
    if env.name.trim().is_empty() {
        errors.push("`name` must not be empty".into());
    }
    if let Some(config) = &env.config {
        if !config.is_object() {
            errors.push("`config` must be a JSON object".into());
        }
    }

    let resolved = match (&env.spec, &env.source) {
        (Some(_), Some(_)) => {
            errors.push("provide exactly one of `spec` or `source`, not both".into());
            None
        }
        (None, None) => {
            errors.push("provide one of `spec` (an Expr tree) or `source` (lemon text)".into());
            None
        }
        (Some(spec), None) => match serde_json::from_value::<Expr>(spec.clone()) {
            Ok(_) => Some(spec.clone()),
            Err(e) => {
                errors.push(format!("`spec` is not a valid strategy tree: {e}"));
                None
            }
        },
        (None, Some(src)) => match crate::parse(src) {
            Ok(tree) => Some(tree),
            Err(e) => {
                errors.push(format!(
                    "`source` failed to parse: {}:{}: {}",
                    e.line, e.col, e.message
                ));
                None
            }
        },
    };

    match (errors.is_empty(), resolved) {
        (true, Some(spec)) => Ok(Checked {
            name: env.name,
            spec,
        }),
        _ => Err(errors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_a_source_envelope_and_resolves_the_spec() {
        let doc = r#"{
            "format": 1,
            "name": "Momentum",
            "description": "buy strength",
            "source": "close > sma(close, 2)",
            "config": { "fee_ratio": 0.001 },
            "universe": { "from": 20180101, "to": 20251231, "symbols_hint": "sp500" },
            "engine_version": "yuzu-core 0.x"
        }"#;
        let checked = check(doc).expect("valid envelope");
        assert_eq!(checked.name, "Momentum");
        // Resolved spec is exactly what the parser produces for the source.
        assert_eq!(checked.spec, crate::parse("close > sma(close, 2)").unwrap());
        assert_eq!(checked.spec["op"], "Gt");
    }

    #[test]
    fn checks_a_spec_envelope() {
        let spec = crate::parse("rsi(close, 14)").unwrap();
        let doc = serde_json::json!({
            "format": 1,
            "name": "RSI",
            "spec": spec,
        })
        .to_string();
        let checked = check(&doc).expect("valid spec envelope");
        assert_eq!(checked.spec["op"], "Rsi");
    }

    #[test]
    fn rejects_wrong_format_version() {
        let doc = r#"{ "format": 99, "name": "X", "source": "close" }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("unsupported envelope format 99")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_empty_name() {
        let doc = r#"{ "format": 1, "name": "  ", "source": "close" }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("`name` must not be empty")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_both_spec_and_source() {
        let doc = r#"{ "format": 1, "name": "X", "source": "close", "spec": {"op":"Data","name":"close"} }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("exactly one of `spec` or `source`")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_neither_spec_nor_source() {
        let doc = r#"{ "format": 1, "name": "X" }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("provide one of `spec`")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_a_malformed_spec_tree() {
        // `Average` requires an `n` field; omitting it must be caught.
        let doc = r#"{ "format": 1, "name": "X",
            "spec": { "op": "Average", "of": { "op": "Data", "name": "close" } } }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("not a valid strategy tree")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_an_unknown_op_tag() {
        let doc = r#"{ "format": 1, "name": "X",
            "spec": { "op": "Nonesuch", "of": { "op": "Data", "name": "close" } } }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("not a valid strategy tree")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_a_source_syntax_error() {
        let doc = r#"{ "format": 1, "name": "X", "source": "sma(close," }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("`source` failed to parse")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_unknown_envelope_keys() {
        let doc = r#"{ "format": 1, "name": "X", "source": "close", "descroption": "typo" }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("not a valid strategy envelope")),
            "{errs:?}"
        );
    }

    #[test]
    fn rejects_non_object_config() {
        let doc = r#"{ "format": 1, "name": "X", "source": "close", "config": 3 }"#;
        let errs = check(doc).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("`config` must be a JSON object")),
            "{errs:?}"
        );
    }

    #[test]
    fn checked_in_schema_tracks_the_format_version() {
        // Guard against the committed JSON Schema drifting from the code: its
        // `format.const` must equal FORMAT_VERSION. CARGO_MANIFEST_DIR is
        // <workspace>/crates/lemon-lang, so schema/ is two levels up.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("schema/strategy-envelope.schema.json");
        let body = std::fs::read_to_string(&path).expect("read envelope schema");
        let schema: Value = serde_json::from_str(&body).expect("schema is valid JSON");
        assert_eq!(
            schema["properties"]["format"]["const"].as_u64(),
            Some(FORMAT_VERSION as u64),
            "schema/strategy-envelope.schema.json format const != FORMAT_VERSION",
        );
    }
}
