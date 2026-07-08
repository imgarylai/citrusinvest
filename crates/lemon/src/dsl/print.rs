//! JSON `Expr` tree → DSL text (no `let` reconstruction). A call breaks onto
//! indented lines only when one of its arguments is itself nested (a call or
//! binop); leaves and flat calls like `sma(close, 2)` stay on one line.

use serde_json::Value;

use super::ops::{self, Field};

const INDENT: &str = "  ";

pub fn format(expr: &Value) -> String {
    format_at(expr, 0)
}

/// `depth` is the indentation level of the node's *own* opening line; nested
/// calls that break indent their args at `depth + 1`.
fn format_at(expr: &Value, depth: usize) -> String {
    let Some(tag) = expr.get("op").and_then(Value::as_str) else {
        return literal(expr);
    };
    match tag {
        "Data" => expr
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string(),
        "Const" => literal(expr.get("value").unwrap_or(&Value::Null)),
        "Neg" => format!("(-{})", format_at(child(expr, "of"), depth)),
        "Not" => format!("(not {})", format_at(child(expr, "of"), depth)),
        "Ceil" => format!("ceil({})", format_at(child(expr, "of"), depth)),
        _ => {
            if let Some(sym) = ops::binop_symbol_for_tag(tag) {
                // Binops stay inline; a nested call inside one still breaks itself.
                return format!(
                    "({} {} {})",
                    format_at(child(expr, "l"), depth),
                    sym,
                    format_at(child(expr, "r"), depth)
                );
            }
            print_call(tag, expr, depth)
        }
    }
}

/// A node forces its parent call to break iff it isn't a bare leaf (`Data`,
/// `Const`, or a scalar literal) — i.e. it's a call or binop.
fn is_leaf(v: &Value) -> bool {
    !matches!(v.get("op").and_then(Value::as_str), Some(tag) if tag != "Data" && tag != "Const")
}

fn print_call(tag: &str, expr: &Value, depth: usize) -> String {
    let name = ops::dsl_name_for_tag(tag);
    let Some(sig) = ops::sig_by_tag(tag) else {
        return format!("{name}(/* unprintable op {tag} */)");
    };
    let child_depth = depth + 1;
    let mut parts: Vec<String> = Vec::new();
    let mut brk = false; // set true once any Expr arg is nested → break this call
                         // Once true, every subsequent field must be emitted as keyword=value so we
                         // never produce a positional arg after a keyword arg (parser would reject it).
    let mut keyword_mode = false;
    for field in sig.fields {
        let fname = ops::field_name(field);
        let Some(val) = expr.get(fname) else {
            // Gap in the positional prefix — everything after must be keyword.
            keyword_mode = true;
            continue;
        };
        let rendered = match field {
            Field::ExprList(_) => {
                let items = val.as_array().cloned().unwrap_or_default();
                if items.iter().any(|v| !is_leaf(v)) {
                    brk = true;
                }
                let rendered: Vec<String> =
                    items.iter().map(|v| format_at(v, child_depth)).collect();
                format!("[{}]", rendered.join(", "))
            }
            Field::StrListOpt(_) => {
                let empty = Vec::new();
                let items: Vec<String> = val
                    .as_array()
                    .unwrap_or(&empty)
                    .iter()
                    .map(literal)
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Field::Expr(_) | Field::ExprOpt(_) => {
                if !is_leaf(val) {
                    brk = true;
                }
                format_at(val, child_depth)
            }
            _ => literal(val),
        };
        // A field is positional only when we haven't entered keyword mode yet AND
        // the field is a required scalar/Expr (i.e. Expr, Num, or Str — NOT any
        // *Opt or ExprList variant, which have no positional slot in the grammar).
        let is_required_inline = matches!(field, Field::Expr(_) | Field::Num(_) | Field::Str(_));
        if !keyword_mode && is_required_inline {
            parts.push(rendered);
        } else {
            keyword_mode = true;
            parts.push(format!("{fname}={rendered}"));
        }
    }
    if brk {
        let ind = INDENT.repeat(child_depth);
        let close = INDENT.repeat(depth);
        format!(
            "{name}(\n{ind}{}\n{close})",
            parts.join(&format!(",\n{ind}"))
        )
    } else {
        format!("{name}({})", parts.join(", "))
    }
}

fn child<'a>(expr: &'a Value, key: &str) -> &'a Value {
    expr.get(key).unwrap_or(&Value::Null)
}

fn literal(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => format!("\"{s}\""),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prints_leaves_and_call() {
        assert_eq!(format(&json!({"op":"Data","name":"close"})), "close");
        assert_eq!(format(&json!({"op":"Const","value":1000000})), "1000000");
        assert_eq!(
            format(&json!({"op":"Average","of":{"op":"Data","name":"close"},"n":2})),
            "sma(close, 2)"
        );
    }

    #[test]
    fn prints_not_and_round_trips() {
        let v = json!({"op":"Not","of":{"op":"Gt",
            "l":{"op":"Data","name":"a"},"r":{"op":"Const","value":1}}});
        assert_eq!(format(&v), "(not (a > 1))");
        assert_eq!(super::super::parse(&format(&v)).unwrap(), v);
    }

    #[test]
    fn prints_binop_parenthesised() {
        assert_eq!(
            format(&json!({"op":"Gt","l":{"op":"Data","name":"close"},
                          "r":{"op":"Const","value":2}})),
            "(close > 2)"
        );
    }

    #[test]
    fn prints_keyword_fields_and_list() {
        assert_eq!(
            format(&json!({"op":"Rebalance","of":{"op":"Data","name":"x"},"freq":"ME"})),
            "rebalance(x, freq=\"ME\")"
        );
        assert_eq!(
            format(&json!({"op":"Neutralize","of":{"op":"Data","name":"x"},
                          "by":[{"op":"Data","name":"pe"}]})),
            "neutralize(x, by=[pe])"
        );
    }

    #[test]
    fn vwap_prints_all_positional_and_reparses() {
        let v = serde_json::json!({"op":"Vwap","high":{"op":"Data","name":"high"},
            "low":{"op":"Data","name":"low"},"close":{"op":"Data","name":"close"},
            "volume":{"op":"Data","name":"volume"},"n":20});
        assert_eq!(format(&v), "vwap(high, low, close, volume, 20)");
        assert_eq!(super::super::parse(&format(&v)).unwrap(), v);
    }

    #[test]
    fn rank_with_gapped_optional_uses_keyword() {
        let v = serde_json::json!({"op":"Rank","of":{"op":"Data","name":"x"},"ascending":false});
        assert_eq!(format(&v), "rank(x, ascending=false)");
        assert_eq!(super::super::parse(&format(&v)).unwrap(), v);
    }

    #[test]
    fn nested_call_breaks_with_indent_flat_stays_inline() {
        // rebalance wraps is_largest wraps sma. sma's args are leaves → it stays
        // flat; its two enclosing calls each have a nested arg → both break.
        let v = json!({"op":"Rebalance","freq":"ME",
            "of":{"op":"IsLargest","n":10,
                  "of":{"op":"Average","of":{"op":"Data","name":"close"},"n":5}}});
        assert_eq!(
            format(&v),
            "rebalance(\n  is_largest(\n    sma(close, 5),\n    10\n  ),\n  freq=\"ME\"\n)"
        );
        assert_eq!(super::super::parse(&format(&v)).unwrap(), v);
    }

    #[test]
    fn binop_arg_breaks_parent_but_stays_inline_itself() {
        let v = json!({"op":"Mask","of":{"op":"IsLargest","of":{"op":"Data","name":"x"},"n":30},
            "by":{"op":"Gt","l":{"op":"Data","name":"market_cap"},"r":{"op":"Const","value":5}}});
        assert_eq!(
            format(&v),
            "mask(\n  is_largest(x, 30),\n  (market_cap > 5)\n)"
        );
        assert_eq!(super::super::parse(&format(&v)).unwrap(), v);
    }
}
