//! WASM boundary for the Lemon DSL: parse `.lemon` source ⇄ JSON `Expr` tree.
//! String in, string out — mirrors `yuzu-wasm`'s JSON boundary. The pure
//! functions are unit-tested natively; the `#[wasm_bindgen]` wrappers are
//! wasm32-gated.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use serde_json::{json, Value};

/// Parse Lemon source to a tagged-result JSON string. NEVER throws — the editor
/// inspects `ok` and reads `error.line`/`error.col` for live diagnostics.
/// success: `{"ok":true,"value":<expr>}`
/// failure: `{"ok":false,"error":{"line":L,"col":C,"message":"..."}}`
pub fn parse_to_json(src: &str) -> String {
    match lemon::parse(src) {
        Ok(value) => json!({ "ok": true, "value": value }).to_string(),
        Err(e) => json!({
            "ok": false,
            "error": { "line": e.line, "col": e.col, "message": e.message }
        })
        .to_string(),
    }
}

/// Render a JSON `Expr` tree (as a string) back to Lemon source. Errors only on
/// malformed input JSON (defensive — `format` is called on engine-produced JSON).
pub fn format_from_json(json_str: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    Ok(lemon::format(&value))
}

/// Lint Lemon source to a tagged-result JSON string. NEVER throws.
/// `series_json` is a JSON array of known series names (`["close","pe",…]`);
/// pass `null` (or invalid JSON) to skip the unknown-series check.
/// success: `{"ok":true,"lints":[{"line":L,"col":C,"message":"..."}]}`
/// parse failure: `{"ok":false,"error":{"line":L,"col":C,"message":"..."}}`
pub fn lint_to_json(src: &str, series_json: &str) -> String {
    let series: Option<Vec<String>> = serde_json::from_str(series_json).ok().flatten();
    match lemon::lint(src, series.as_deref()) {
        Ok(lints) => {
            let items: Vec<Value> = lints
                .iter()
                .map(|l| json!({ "line": l.line, "col": l.col, "message": l.message }))
                .collect();
            json!({ "ok": true, "lints": items }).to_string()
        }
        Err(e) => json!({
            "ok": false,
            "error": { "line": e.line, "col": e.col, "message": e.message }
        })
        .to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn parse(src: &str) -> String {
    parse_to_json(src)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn lint(src: &str, series_json: &str) -> String {
    lint_to_json(src, series_json)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn format(json_str: &str) -> Result<String, JsValue> {
    format_from_json(json_str).map_err(|e| JsValue::from_str(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_success_is_tagged_ok() {
        let out: Value = serde_json::from_str(&parse_to_json("close > sma(close, 2)")).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["value"]["op"], "Gt");
    }

    #[test]
    fn parse_error_is_tagged_with_position() {
        // bare constant is rejected by lemon::parse
        let out: Value = serde_json::from_str(&parse_to_json("42")).unwrap();
        assert_eq!(out["ok"], false);
        assert!(out["error"]["message"].is_string());
        assert!(out["error"]["line"].is_number());
        assert!(out["error"]["col"].is_number());
    }

    #[test]
    fn lint_is_tagged_and_null_series_skips_unknown_check() {
        let out: Value =
            serde_json::from_str(&lint_to_json("clsoe > 1", r#"["close","pe"]"#)).unwrap();
        assert_eq!(out["ok"], true);
        let lints = out["lints"].as_array().unwrap();
        assert_eq!(lints.len(), 1);
        assert!(lints[0]["message"]
            .as_str()
            .unwrap()
            .contains("did you mean `close`"));
        // null series -> unknown-series check off
        let out: Value = serde_json::from_str(&lint_to_json("clsoe > 1", "null")).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["lints"].as_array().unwrap().len(), 0);
        // parse error -> tagged error
        let out: Value = serde_json::from_str(&lint_to_json("sma(close,", "null")).unwrap();
        assert_eq!(out["ok"], false);
        assert!(out["error"]["line"].is_number());
    }

    #[test]
    fn format_renders_tree_to_source() {
        let tree = r#"{"op":"Average","of":{"op":"Data","name":"close"},"n":2}"#;
        assert_eq!(format_from_json(tree).unwrap(), "sma(close, 2)");
    }

    #[test]
    fn format_rejects_malformed_json() {
        assert!(format_from_json("{not json").is_err());
    }
}
