//! Op vocabulary: DSL surface names ⇄ JSON `Expr` op tags + field layout.
//! Field declaration order MUST match `spec.rs` so positional args line up.

#[derive(Debug, Clone, Copy)]
pub enum Field {
    Expr(&'static str),
    ExprOpt(&'static str),
    ExprList(&'static str),
    Num(&'static str),
    NumOpt(&'static str),
    BoolOpt(&'static str),
    Str(&'static str),
    StrOpt(&'static str),
    StrListOpt(&'static str),
}

pub fn field_name(f: &Field) -> &'static str {
    match f {
        Field::Expr(n) | Field::ExprOpt(n) | Field::ExprList(n) | Field::Num(n)
        | Field::NumOpt(n) | Field::BoolOpt(n) | Field::Str(n) | Field::StrOpt(n)
        | Field::StrListOpt(n) => n,
    }
}

pub struct OpSig {
    pub tag: &'static str,
    pub fields: &'static [Field],
}

/// One row per function-style op. `names[0]` is the canonical DSL name (used by the
/// printer); the rest are accepted aliases. Operator-style ops (Gt/And/Add/Neg/…)
/// are NOT here — see `binop_tag`/`prefix_tag`.
struct Row {
    names: &'static [&'static str],
    sig: OpSig,
}

use Field::*;

static ROWS: &[Row] = &[
    Row { names: &["sma", "average"], sig: OpSig { tag: "Average", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["ema"], sig: OpSig { tag: "Ema", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["std"], sig: OpSig { tag: "Std", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["rsi"], sig: OpSig { tag: "Rsi", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["pct_change"], sig: OpSig { tag: "PctChange", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["rise"], sig: OpSig { tag: "Rise", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["fall"], sig: OpSig { tag: "Fall", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["shift"], sig: OpSig { tag: "Shift", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["rolling_max"], sig: OpSig { tag: "RollingMax", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["atr"], sig: OpSig { tag: "Atr", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["natr"], sig: OpSig { tag: "Natr", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["willr"], sig: OpSig { tag: "WillR", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["cci"], sig: OpSig { tag: "Cci", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["stoch_k"], sig: OpSig { tag: "StochK", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["stoch_d"], sig: OpSig { tag: "StochD", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n"), NumOpt("d")] } },
    Row { names: &["aroon_up"], sig: OpSig { tag: "AroonUp", fields: &[Expr("high"), Num("n")] } },
    Row { names: &["aroon_down"], sig: OpSig { tag: "AroonDown", fields: &[Expr("low"), Num("n")] } },
    Row { names: &["adx"], sig: OpSig { tag: "Adx", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["plus_di"], sig: OpSig { tag: "PlusDi", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["minus_di"], sig: OpSig { tag: "MinusDi", fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")] } },
    Row { names: &["obv"], sig: OpSig { tag: "Obv", fields: &[Expr("close"), Expr("volume")] } },
    Row { names: &["mfi"], sig: OpSig { tag: "Mfi", fields: &[Expr("high"), Expr("low"), Expr("close"), Expr("volume"), Num("n")] } },
    Row { names: &["vwap"], sig: OpSig { tag: "Vwap", fields: &[Expr("high"), Expr("low"), Expr("close"), Expr("volume"), Num("n")] } },
    Row { names: &["is_largest"], sig: OpSig { tag: "IsLargest", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["is_smallest"], sig: OpSig { tag: "IsSmallest", fields: &[Expr("of"), Num("n")] } },
    Row { names: &["sustain"], sig: OpSig { tag: "Sustain", fields: &[Expr("of"), Num("nwindow"), NumOpt("nsatisfy")] } },
    Row { names: &["is_entry"], sig: OpSig { tag: "IsEntry", fields: &[Expr("of")] } },
    Row { names: &["is_exit"], sig: OpSig { tag: "IsExit", fields: &[Expr("of")] } },
    Row { names: &["ceil"], sig: OpSig { tag: "Ceil", fields: &[Expr("of")] } },
    Row { names: &["rank"], sig: OpSig { tag: "Rank", fields: &[Expr("of"), BoolOpt("pct"), BoolOpt("ascending")] } },
    Row { names: &["mask"], sig: OpSig { tag: "Mask", fields: &[Expr("of"), Expr("by")] } },
    Row {
        names: &["hold_until"],
        sig: OpSig {
            tag: "HoldUntil",
            fields: &[
                Expr("entry"), Expr("exit"), NumOpt("nstocks_limit"), ExprOpt("rank"),
                NumOpt("stop_loss"), NumOpt("take_profit"), NumOpt("trail_stop"), NumOpt("trail_stop_activation"),
            ],
        },
    },
    Row { names: &["rebalance"], sig: OpSig { tag: "Rebalance", fields: &[Expr("of"), StrOpt("freq"), ExprOpt("on")] } },
    Row { names: &["neutralize"], sig: OpSig { tag: "Neutralize", fields: &[Expr("of"), ExprList("by"), BoolOpt("add_const")] } },
    Row { names: &["neutralize_industry"], sig: OpSig { tag: "NeutralizeIndustry", fields: &[Expr("of"), BoolOpt("add_const")] } },
    Row { names: &["industry_rank"], sig: OpSig { tag: "IndustryRank", fields: &[Expr("of"), StrListOpt("categories")] } },
    Row { names: &["groupby_category"], sig: OpSig { tag: "GroupbyCategory", fields: &[Expr("of"), Str("agg")] } },
];

pub fn op_by_name(name: &str) -> Option<&'static OpSig> {
    ROWS.iter().find(|r| r.names.contains(&name)).map(|r| &r.sig)
}

pub fn sig_by_tag(tag: &str) -> Option<&'static OpSig> {
    ROWS.iter().find(|r| r.sig.tag == tag).map(|r| &r.sig)
}

/// Canonical DSL name for a function-style op tag (printer).
pub fn dsl_name_for_tag(tag: &str) -> &'static str {
    ROWS.iter()
        .find(|r| r.sig.tag == tag)
        .map(|r| r.names[0])
        .unwrap_or_else(|| {
            // Tag not in the function table — callers should not reach this for
            // operator-style ops; return the tag itself via a leaked copy so the
            // return type stays `&'static str`.
            ALL_OP_TAGS.iter().copied().find(|t| *t == tag).unwrap_or("")
        })
}

static BINOPS: &[(&str, &str)] = &[
    (">", "Gt"), ("<", "Lt"), (">=", "Ge"), ("<=", "Le"),
    ("and", "And"), ("or", "Or"),
    ("+", "Add"), ("-", "Sub"), ("*", "Mul"), ("/", "Div"),
];

pub fn binop_tag(op: &str) -> Option<&'static str> {
    BINOPS.iter().find(|(s, _)| *s == op).map(|(_, t)| *t)
}

pub fn binop_symbol_for_tag(tag: &str) -> Option<&'static str> {
    BINOPS.iter().find(|(_, t)| *t == tag).map(|(s, _)| *s)
}

pub fn prefix_tag(op: &str) -> Option<&'static str> {
    if op == "-" { Some("Neg") } else { None }
}

/// All op tags in `spec.rs` — the completeness gate (see ops::tests).
pub static ALL_OP_TAGS: &[&str] = &[
    "Data", "Const", "Average", "Ema", "Std", "Rsi", "PctChange", "Rise", "Shift",
    "RollingMax", "Atr", "Natr", "WillR", "Cci", "StochK", "StochD", "AroonUp",
    "AroonDown", "Adx", "PlusDi", "MinusDi", "Obv", "Mfi", "Vwap", "Fall",
    "IsLargest", "IsSmallest", "Sustain", "IsEntry", "IsExit", "Gt", "Lt", "Ge",
    "Le", "And", "Or", "Add", "Sub", "Mul", "Div", "Neg", "Ceil", "Rank", "Mask",
    "HoldUntil", "Rebalance", "Neutralize", "NeutralizeIndustry", "IndustryRank",
    "GroupbyCategory",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_alias_and_snake_case() {
        assert_eq!(op_by_name("sma").unwrap().tag, "Average");
        assert_eq!(op_by_name("average").unwrap().tag, "Average");
        assert_eq!(op_by_name("rolling_max").unwrap().tag, "RollingMax");
        assert!(op_by_name("not_an_op").is_none());
    }

    #[test]
    fn maps_operators() {
        assert_eq!(binop_tag(">"), Some("Gt"));
        assert_eq!(binop_tag("and"), Some("And"));
        assert_eq!(binop_tag("+"), Some("Add"));
        assert_eq!(prefix_tag("-"), Some("Neg"));
    }

    #[test]
    fn round_trip_names_for_printer() {
        assert_eq!(dsl_name_for_tag("Average"), "sma");
        assert_eq!(binop_symbol_for_tag("Gt"), Some(">"));
        assert_eq!(binop_symbol_for_tag("Average"), None);
    }

    #[test]
    fn every_spec_op_has_a_signature_or_operator() {
        // The 51 op tags from spec.rs. If a new op is added, add it here AND to the
        // table/operator maps — this test is the completeness gate for this plan.
        for tag in ALL_OP_TAGS {
            let known = ROWS.iter().any(|r| r.sig.tag == *tag)
                || binop_symbol_for_tag(tag).is_some()
                || *tag == "Neg"
                || *tag == "Const"
                || *tag == "Data";
            assert!(known, "op `{tag}` has no DSL handler");
        }
    }
}
