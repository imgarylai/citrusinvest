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
        Field::Expr(n)
        | Field::ExprOpt(n)
        | Field::ExprList(n)
        | Field::Num(n)
        | Field::NumOpt(n)
        | Field::BoolOpt(n)
        | Field::Str(n)
        | Field::StrOpt(n)
        | Field::StrListOpt(n) => n,
    }
}

/// JSON-schema kind for a field: `expr` (an `Expr` subtree), `number`, `string`,
/// `bool`, or `list` (a list of `Expr`) / `list-string`.
pub fn field_kind(f: &Field) -> &'static str {
    match f {
        Field::Expr(_) | Field::ExprOpt(_) => "expr",
        Field::ExprList(_) => "list",
        Field::Num(_) | Field::NumOpt(_) => "number",
        Field::BoolOpt(_) => "bool",
        Field::Str(_) | Field::StrOpt(_) => "string",
        Field::StrListOpt(_) => "list-string",
    }
}

/// Whether the parser requires this field (required scalar/Expr/list) or accepts
/// it as optional (`*Opt` variants).
pub fn field_required(f: &Field) -> bool {
    matches!(
        f,
        Field::Expr(_) | Field::Num(_) | Field::Str(_) | Field::ExprList(_)
    )
}

pub struct OpSig {
    pub tag: &'static str,
    pub fields: &'static [Field],
}

/// One row per function-style op. `names[0]` is the canonical DSL name (used by the
/// printer); the rest are accepted aliases. Operator-style ops (Gt/And/Add/Neg/…)
/// are NOT here — see `binop_tag`/`prefix_tag`.
///
/// `desc` is a one-line, machine-readable description consumed by the schema
/// generator (`cargo run -p lemon-lang --example gen-schema`); keep it terse and
/// in sync with the `Expr` doc-comment in `spec.rs`.
struct Row {
    names: &'static [&'static str],
    sig: OpSig,
    desc: &'static str,
}

use Field::*;

static ROWS: &[Row] = &[
    Row {
        names: &["sma", "average"],
        sig: OpSig {
            tag: "Average",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Simple moving average of `of` over `n` days.",
    },
    Row {
        names: &["ema"],
        sig: OpSig {
            tag: "Ema",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Exponential moving average of `of` over `n` days.",
    },
    Row {
        names: &["std"],
        sig: OpSig {
            tag: "Std",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Rolling standard deviation of `of` over `n` days.",
    },
    Row {
        names: &["rsi"],
        sig: OpSig {
            tag: "Rsi",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Relative Strength Index of `of` over `n` days.",
    },
    Row {
        names: &["pct_change"],
        sig: OpSig {
            tag: "PctChange",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Percentage change of `of` over `n` days.",
    },
    Row {
        names: &["rise"],
        sig: OpSig {
            tag: "Rise",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "1 where `of` rose for `n` consecutive days, else 0.",
    },
    Row {
        names: &["fall"],
        sig: OpSig {
            tag: "Fall",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "1 where `of` fell for `n` consecutive days, else 0.",
    },
    Row {
        names: &["shift"],
        sig: OpSig {
            tag: "Shift",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "`of` lagged forward by `n` days.",
    },
    Row {
        names: &["rolling_max"],
        sig: OpSig {
            tag: "RollingMax",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Rolling maximum of `of` over `n` days.",
    },
    Row {
        names: &["atr"],
        sig: OpSig {
            tag: "Atr",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Average True Range over `n` days from high/low/close.",
    },
    Row {
        names: &["natr"],
        sig: OpSig {
            tag: "Natr",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Normalized ATR (percent) over `n` days.",
    },
    Row {
        names: &["willr"],
        sig: OpSig {
            tag: "WillR",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Williams %R over `n` days.",
    },
    Row {
        names: &["cci"],
        sig: OpSig {
            tag: "Cci",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Commodity Channel Index over `n` days.",
    },
    Row {
        names: &["stoch_k"],
        sig: OpSig {
            tag: "StochK",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Stochastic %K over `n` days.",
    },
    Row {
        names: &["stoch_d"],
        sig: OpSig {
            tag: "StochD",
            fields: &[
                Expr("high"),
                Expr("low"),
                Expr("close"),
                Num("n"),
                NumOpt("d"),
            ],
        },
        desc: "Stochastic %D: `d`-day average of %K over `n` days (d defaults to 3).",
    },
    Row {
        names: &["aroon_up"],
        sig: OpSig {
            tag: "AroonUp",
            fields: &[Expr("high"), Num("n")],
        },
        desc: "Aroon Up over `n` days from high.",
    },
    Row {
        names: &["aroon_down"],
        sig: OpSig {
            tag: "AroonDown",
            fields: &[Expr("low"), Num("n")],
        },
        desc: "Aroon Down over `n` days from low.",
    },
    Row {
        names: &["adx"],
        sig: OpSig {
            tag: "Adx",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Average Directional Index over `n` days.",
    },
    Row {
        names: &["plus_di"],
        sig: OpSig {
            tag: "PlusDi",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Plus Directional Indicator (+DI) over `n` days.",
    },
    Row {
        names: &["minus_di"],
        sig: OpSig {
            tag: "MinusDi",
            fields: &[Expr("high"), Expr("low"), Expr("close"), Num("n")],
        },
        desc: "Minus Directional Indicator (-DI) over `n` days.",
    },
    Row {
        names: &["obv"],
        sig: OpSig {
            tag: "Obv",
            fields: &[Expr("close"), Expr("volume")],
        },
        desc: "On-Balance Volume from close and volume.",
    },
    Row {
        names: &["mfi"],
        sig: OpSig {
            tag: "Mfi",
            fields: &[
                Expr("high"),
                Expr("low"),
                Expr("close"),
                Expr("volume"),
                Num("n"),
            ],
        },
        desc: "Money Flow Index over `n` days.",
    },
    Row {
        names: &["vwap"],
        sig: OpSig {
            tag: "Vwap",
            fields: &[
                Expr("high"),
                Expr("low"),
                Expr("close"),
                Expr("volume"),
                Num("n"),
            ],
        },
        desc: "Volume-Weighted Average Price over `n` days from high/low/close/volume.",
    },
    Row {
        names: &["is_largest"],
        sig: OpSig {
            tag: "IsLargest",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "1 for the `n` highest values per row (cross-section), else 0.",
    },
    Row {
        names: &["is_smallest"],
        sig: OpSig {
            tag: "IsSmallest",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "1 for the `n` lowest values per row (cross-section), else 0.",
    },
    Row {
        names: &["sustain"],
        sig: OpSig {
            tag: "Sustain",
            fields: &[Expr("of"), Num("nwindow"), NumOpt("nsatisfy")],
        },
        desc: "1 where `of` held true at least `nsatisfy` times within the last `nwindow` rows.",
    },
    Row {
        names: &["is_entry"],
        sig: OpSig {
            tag: "IsEntry",
            fields: &[Expr("of")],
        },
        desc: "1 on the row where `of` turns false->true (rising edge).",
    },
    Row {
        names: &["is_exit"],
        sig: OpSig {
            tag: "IsExit",
            fields: &[Expr("of")],
        },
        desc: "1 on the row where `of` turns true->false (falling edge).",
    },
    Row {
        names: &["exit_when"],
        sig: OpSig {
            tag: "ExitWhen",
            fields: &[Expr("entry"), Expr("exit")],
        },
        desc: "Hold true from an entry edge of `entry` until an exit edge (or `exit` is true).",
    },
    Row {
        names: &["quantile_row"],
        sig: OpSig {
            tag: "QuantileRow",
            fields: &[Expr("of"), Num("c")],
        },
        desc: "Per-row quantile of `of` across symbols at level `c` (e.g. 0.5 = median); one-column result.",
    },
    Row {
        names: &["winsorize"],
        sig: OpSig {
            tag: "Winsorize",
            fields: &[Expr("of"), Num("lower"), Num("upper")],
        },
        desc: "Per-row winsorize: clip values to empirical quantiles `lower`/`upper` (in 0..1).",
    },
    Row {
        names: &["zscore"],
        sig: OpSig {
            tag: "Zscore",
            fields: &[Expr("of")],
        },
        desc: "Per-row z-score (population std); NaN preserved; constant rows become 0.",
    },
    Row {
        names: &["bucket"],
        sig: OpSig {
            tag: "Bucket",
            fields: &[Expr("of"), Num("n")],
        },
        desc: "Per-row quantile buckets labeled 1..=n (ties share average rank).",
    },
    Row {
        names: &["demean"],
        sig: OpSig {
            tag: "Demean",
            fields: &[Expr("of")],
        },
        desc: "Per-row demean: subtract the cross-sectional mean of non-NaN cells.",
    },
    Row {
        names: &["ceil"],
        sig: OpSig {
            tag: "Ceil",
            fields: &[Expr("of")],
        },
        desc: "Ceiling of `of`.",
    },
    Row {
        names: &["rank"],
        sig: OpSig {
            tag: "Rank",
            fields: &[Expr("of"), BoolOpt("pct"), BoolOpt("ascending")],
        },
        desc: "Cross-sectional rank of `of` per row; `pct` for 0..1 percentile (default true), `ascending` sets direction (default true).",
    },
    Row {
        names: &["mask"],
        sig: OpSig {
            tag: "Mask",
            fields: &[Expr("of"), Expr("by")],
        },
        desc: "`of` kept only where `by` is true; elsewhere dropped.",
    },
    Row {
        names: &["normalize_row"],
        sig: OpSig {
            tag: "NormalizeRow",
            fields: &[Expr("of")],
        },
        desc: "Scale each row so gross weight (sum of |w|) is 1 — turns a raw signal into explicit portfolio weights. NaN preserved; zero rows unchanged.",
    },
    Row {
        names: &["hold_until"],
        sig: OpSig {
            tag: "HoldUntil",
            fields: &[
                Expr("entry"),
                Expr("exit"),
                NumOpt("nstocks_limit"),
                ExprOpt("rank"),
                NumOpt("stop_loss"),
                NumOpt("take_profit"),
                NumOpt("trail_stop"),
                NumOpt("trail_stop_activation"),
            ],
        },
        desc: "Stateful rotation: enter on `entry`, exit on `exit`, hold up to `nstocks_limit` (prioritised by `rank`), with optional stop_loss/take_profit/trail_stop/trail_stop_activation.",
    },
    Row {
        names: &["rebalance"],
        sig: OpSig {
            tag: "Rebalance",
            fields: &[Expr("of"), StrOpt("freq"), ExprOpt("on")],
        },
        desc: "Hold `of`, refreshing on calendar `freq` (W/ME/QE) or on dates where `on` is true.",
    },
    Row {
        names: &["neutralize"],
        sig: OpSig {
            tag: "Neutralize",
            fields: &[Expr("of"), ExprList("by"), BoolOpt("add_const")],
        },
        desc: "Cross-sectionally regress `of` against the `by` factors, optionally adding a constant (default true).",
    },
    Row {
        names: &["neutralize_industry"],
        sig: OpSig {
            tag: "NeutralizeIndustry",
            fields: &[Expr("of"), BoolOpt("add_const")],
        },
        desc: "Neutralize `of` within each industry/sector (add_const defaults to true).",
    },
    Row {
        names: &["industry_rank"],
        sig: OpSig {
            tag: "IndustryRank",
            fields: &[Expr("of"), StrListOpt("categories")],
        },
        desc: "Rank `of` within each industry, optionally limited to `categories`.",
    },
    Row {
        names: &["groupby_category"],
        sig: OpSig {
            tag: "GroupbyCategory",
            fields: &[Expr("of"), Str("agg")],
        },
        desc: "Aggregate `of` within each industry using `agg` (e.g. mean).",
    },
    Row {
        names: &["in_sector"],
        sig: OpSig {
            tag: "InSector",
            fields: &[Expr("of"), Str("name")],
        },
        desc: "Boolean mask (1/0) where the symbol's industry equals `name` (exact, case-sensitive); shape follows `of`.",
    },
];

// ---------------------------------------------------------------------------
// Public catalog API — the single source of truth consumed by the schema
// generator (`cargo run -p lemon-lang --example gen-schema`). Everything below
// is derived from `ROWS`, `BINOPS`, and `prefix_tag`, so the emitted JSON
// artifacts can never drift from the parser.
// ---------------------------------------------------------------------------

/// A field's serde default, when it has one. `serde(default)` on an `Option`
/// field means "absent" (no default value to emit); those return `None` here.
/// Kept co-located with the field declarations so it stays in sync with
/// `spec.rs`.
pub fn field_default(tag: &str, field_name: &str) -> Option<serde_json::Value> {
    use serde_json::json;
    match (tag, field_name) {
        ("StochD", "d") => Some(json!(3)),
        ("Rank", "pct") => Some(json!(true)),
        ("Rank", "ascending") => Some(json!(true)),
        ("Neutralize", "add_const") => Some(json!(true)),
        ("NeutralizeIndustry", "add_const") => Some(json!(true)),
        _ => None,
    }
}

/// One field of a callable op, in schema-friendly form.
pub struct FieldInfo {
    pub name: &'static str,
    /// One of: `expr`, `number`, `string`, `bool`, `list`, `list-string`.
    pub kind: &'static str,
    pub required: bool,
    pub default: Option<serde_json::Value>,
}

/// One callable op: its canonical name, aliases, op tag, ordered fields, and a
/// one-line description.
pub struct OpInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub tag: &'static str,
    pub description: &'static str,
    pub fields: Vec<FieldInfo>,
}

/// Every function-style op in `ROWS`, in declaration order.
pub fn function_ops() -> Vec<OpInfo> {
    ROWS.iter()
        .map(|r| OpInfo {
            name: r.names[0],
            aliases: &r.names[1..],
            tag: r.sig.tag,
            description: r.desc,
            fields: r
                .sig
                .fields
                .iter()
                .map(|f| {
                    let name = field_name(f);
                    FieldInfo {
                        name,
                        kind: field_kind(f),
                        required: field_required(f),
                        default: field_default(r.sig.tag, name),
                    }
                })
                .collect(),
        })
        .collect()
}

/// The binary operator ops: `(symbol, op tag)`. Both operands are `l`/`r` exprs.
pub fn binary_operators() -> &'static [(&'static str, &'static str)] {
    BINOPS
}

pub fn op_by_name(name: &str) -> Option<&'static OpSig> {
    ROWS.iter()
        .find(|r| r.names.contains(&name))
        .map(|r| &r.sig)
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
            ALL_OP_TAGS
                .iter()
                .copied()
                .find(|t| *t == tag)
                .unwrap_or("")
        })
}

static BINOPS: &[(&str, &str)] = &[
    (">", "Gt"),
    ("<", "Lt"),
    (">=", "Ge"),
    ("<=", "Le"),
    ("and", "And"),
    ("or", "Or"),
    ("+", "Add"),
    ("-", "Sub"),
    ("*", "Mul"),
    ("/", "Div"),
];

pub fn binop_tag(op: &str) -> Option<&'static str> {
    BINOPS.iter().find(|(s, _)| *s == op).map(|(_, t)| *t)
}

pub fn binop_symbol_for_tag(tag: &str) -> Option<&'static str> {
    BINOPS.iter().find(|(_, t)| *t == tag).map(|(s, _)| *s)
}

pub fn prefix_tag(op: &str) -> Option<&'static str> {
    if op == "-" {
        Some("Neg")
    } else {
        None
    }
}

/// All op tags in `spec.rs` — the completeness gate (see ops::tests).
pub static ALL_OP_TAGS: &[&str] = &[
    "Data",
    "Const",
    "Average",
    "Ema",
    "Std",
    "Rsi",
    "PctChange",
    "Rise",
    "Shift",
    "RollingMax",
    "Atr",
    "Natr",
    "WillR",
    "Cci",
    "StochK",
    "StochD",
    "AroonUp",
    "AroonDown",
    "Adx",
    "PlusDi",
    "MinusDi",
    "Obv",
    "Mfi",
    "Vwap",
    "Fall",
    "IsLargest",
    "IsSmallest",
    "Sustain",
    "IsEntry",
    "IsExit",
    "ExitWhen",
    "QuantileRow",
    "Winsorize",
    "Zscore",
    "Bucket",
    "Demean",
    "Gt",
    "Lt",
    "Ge",
    "Le",
    "And",
    "Or",
    "Not",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Neg",
    "Ceil",
    "Rank",
    "Mask",
    "NormalizeRow",
    "HoldUntil",
    "Rebalance",
    "Neutralize",
    "NeutralizeIndustry",
    "IndustryRank",
    "GroupbyCategory",
    "InSector",
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
    fn function_ops_expose_every_row_with_schema_metadata() {
        let ops = function_ops();
        // One OpInfo per row in the table.
        assert_eq!(ops.len(), ROWS.len());

        // sma is the canonical name for Average and carries `average` as an alias.
        let sma = ops.iter().find(|o| o.tag == "Average").unwrap();
        assert_eq!(sma.name, "sma");
        assert!(sma.aliases.contains(&"average"));
        assert!(!sma.description.is_empty());

        // Every field carries a name, a known kind, and a required flag; the field
        // kinds collectively cover the full `field_kind` match.
        let mut kinds = std::collections::BTreeSet::new();
        for op in &ops {
            for f in &op.fields {
                assert!(!f.name.is_empty());
                assert!(matches!(
                    f.kind,
                    "expr" | "number" | "string" | "bool" | "list" | "list-string"
                ));
                kinds.insert(f.kind);
            }
        }
        // The table exercises expr and number kinds at minimum.
        assert!(kinds.contains("expr"));
        assert!(kinds.contains("number"));
    }

    #[test]
    fn field_defaults_match_serde() {
        // Documented serde defaults surface through field_default…
        assert_eq!(field_default("StochD", "d"), Some(serde_json::json!(3)));
        assert_eq!(field_default("Rank", "pct"), Some(serde_json::json!(true)));
        assert_eq!(
            field_default("Neutralize", "add_const"),
            Some(serde_json::json!(true))
        );
        // …and everything else has no default.
        assert_eq!(field_default("Average", "n"), None);
        assert_eq!(field_default("Nope", "nope"), None);

        // field_default is threaded through function_ops for the defaulted fields.
        let stoch_d = function_ops()
            .into_iter()
            .find(|o| o.tag == "StochD")
            .unwrap();
        let d_field = stoch_d.fields.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(d_field.default, Some(serde_json::json!(3)));
    }

    #[test]
    fn field_kind_and_required_cover_every_variant() {
        use Field::*;
        for (f, kind, required) in [
            (Expr("x"), "expr", true),
            (ExprOpt("x"), "expr", false),
            (ExprList("x"), "list", true),
            (Num("x"), "number", true),
            (NumOpt("x"), "number", false),
            (BoolOpt("x"), "bool", false),
            (Str("x"), "string", true),
            (StrOpt("x"), "string", false),
            (StrListOpt("x"), "list-string", false),
        ] {
            assert_eq!(field_name(&f), "x");
            assert_eq!(field_kind(&f), kind);
            assert_eq!(field_required(&f), required);
        }
    }

    #[test]
    fn tag_and_operator_lookups() {
        // sig_by_tag round-trips against op_by_name.
        assert_eq!(sig_by_tag("Average").unwrap().tag, "Average");
        assert!(sig_by_tag("NotATag").is_none());

        // binary_operators exposes the BINOPS table.
        let ops = binary_operators();
        assert!(ops.contains(&(">", "Gt")));
        assert!(ops.contains(&("/", "Div")));
        assert_eq!(ops.len(), 10);

        // Unknown / operator-style tags return "" from dsl_name_for_tag (fallback arm).
        assert_eq!(dsl_name_for_tag("TotallyUnknownTag"), "");
        assert_eq!(prefix_tag("+"), None);
    }

    #[test]
    fn every_spec_op_has_a_signature_or_operator() {
        // The 51 op tags from spec.rs. If a new op is added, add it here AND to the
        // table/operator maps — this test is the completeness gate for this plan.
        for tag in ALL_OP_TAGS {
            let known = ROWS.iter().any(|r| r.sig.tag == *tag)
                || binop_symbol_for_tag(tag).is_some()
                || *tag == "Neg"
                || *tag == "Not"
                || *tag == "Const"
                || *tag == "Data";
            assert!(known, "op `{tag}` has no DSL handler");
        }
    }
}
