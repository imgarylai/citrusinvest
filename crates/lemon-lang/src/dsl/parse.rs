//! DSL text → JSON `Expr` tree (a `serde_json::Value`).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use super::lex::{lex, Token, TokenKind};
use super::ops::{self, Field};
use super::ParseError;

/// Lower DSL text to a JSON `Expr` tree.
pub fn parse(src: &str) -> Result<Value, ParseError> {
    parse_analyzed(src).map(|a| a.tree)
}

/// A parse plus the source facts the linter needs: where every bare-identifier
/// data-series reference sits, and which `let` bindings were never used.
pub struct Analysis {
    pub tree: Value,
    /// Every `Data` reference: `(name, line, col)`, in source order. Includes
    /// references inside `let` bindings (used or not).
    pub data_refs: Vec<(String, usize, usize)>,
    /// `let` bindings never substituted anywhere: `(name, line, col)`.
    pub unused_lets: Vec<(String, usize, usize)>,
}

/// Like [`parse`], returning the [`Analysis`] side-channel as well.
pub fn parse_analyzed(src: &str) -> Result<Analysis, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks: &toks,
        pos: 0,
        env: HashMap::new(),
        data_refs: Vec::new(),
        let_defs: Vec::new(),
        used_lets: HashSet::new(),
    };
    let tree = p.program()?;
    let unused_lets = p
        .let_defs
        .iter()
        .filter(|(n, _, _)| !p.used_lets.contains(n))
        .cloned()
        .collect();
    Ok(Analysis {
        tree,
        data_refs: p.data_refs,
        unused_lets,
    })
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    env: HashMap<String, Value>,
    data_refs: Vec<(String, usize, usize)>,
    let_defs: Vec<(String, usize, usize)>,
    used_lets: HashSet<String>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn next(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err(&self, message: impl Into<String>) -> ParseError {
        let t = self.peek();
        ParseError {
            line: t.line,
            col: t.col,
            message: message.into(),
        }
    }

    fn program(&mut self) -> Result<Value, ParseError> {
        while self.peek().kind == TokenKind::Let {
            self.next(); // `let`
            let (name, line, col) = match self.next() {
                Token {
                    kind: TokenKind::Ident(n),
                    line,
                    col,
                } => (n, line, col),
                t => {
                    return Err(ParseError {
                        line: t.line,
                        col: t.col,
                        message: "expected name after `let`".into(),
                    })
                }
            };
            if self.next().kind != TokenKind::Eq {
                return Err(self.err("expected `=` in let binding"));
            }
            let value = self.expr(0)?;
            let value = promote(value).map_err(|m| self.err(m))?;
            if self.env.contains_key(&name) {
                return Err(ParseError {
                    line,
                    col,
                    message: format!("`{name}` is already defined"),
                });
            }
            self.let_defs.push((name.clone(), line, col));
            self.env.insert(name, value);
        }

        let v = self.expr(0)?;
        if self.peek().kind != TokenKind::Eof {
            return Err(self.err("unexpected trailing input"));
        }
        if !v.is_object() {
            return Err(self.err("a strategy must be an expression, not a bare constant"));
        }
        Ok(v)
    }

    fn expr(&mut self, min_bp: u8) -> Result<Value, ParseError> {
        // prefix
        let mut lhs = match &self.peek().kind {
            TokenKind::Op(op) => {
                let op = op.clone();
                if let Some(tag) = ops::prefix_tag(&op) {
                    self.next();
                    let operand = self.expr(7)?; // prefix binds tight
                    let of = promote(operand).map_err(|m| self.err(m))?;
                    json!({ "op": tag, "of": of })
                } else {
                    return Err(self.err(format!("unexpected operator `{op}`")));
                }
            }
            // `not` binds looser than comparisons, tighter than `and`:
            // `not a > b` is `not (a > b)`; `not a and b` is `(not a) and b`.
            TokenKind::Ident(s) if s == "not" => {
                self.next();
                let operand = self.expr(5)?;
                let of = promote(operand).map_err(|m| self.err(m))?;
                json!({ "op": "Not", "of": of })
            }
            _ => self.primary()?,
        };

        loop {
            let op = match &self.peek().kind {
                TokenKind::Op(s) => s.clone(),
                TokenKind::Ident(s) if s == "and" || s == "or" => s.clone(),
                _ => break,
            };
            let Some((l_bp, r_bp, tag)) = infix_binding(&op) else {
                break;
            };
            if l_bp < min_bp {
                break;
            }
            self.next(); // consume operator
            let rhs = self.expr(r_bp)?;
            let l = promote(lhs).map_err(|m| self.err(m))?;
            let r = promote(rhs).map_err(|m| self.err(m))?;
            lhs = json!({ "op": tag, "l": l, "r": r });
        }
        Ok(lhs)
    }

    fn primary(&mut self) -> Result<Value, ParseError> {
        let t = self.next();
        match t.kind {
            TokenKind::Num(n) => Ok(number(n)),
            TokenKind::Str(s) => Ok(Value::String(s)),
            TokenKind::LParen => {
                let v = self.expr(0)?;
                if self.next().kind != TokenKind::RParen {
                    return Err(self.err("expected `)`"));
                }
                Ok(v)
            }
            TokenKind::Ident(name) => {
                if name == "true" {
                    return Ok(Value::Bool(true));
                }
                if name == "false" {
                    return Ok(Value::Bool(false));
                }
                if self.peek().kind == TokenKind::LParen {
                    self.call(&name, t.line, t.col)
                } else if let Some(bound) = self.env.get(&name) {
                    self.used_lets.insert(name);
                    Ok(bound.clone())
                } else {
                    self.data_refs.push((name.clone(), t.line, t.col));
                    Ok(json!({ "op": "Data", "name": name }))
                }
            }
            other => Err(ParseError {
                line: t.line,
                col: t.col,
                message: format!("unexpected token {other:?}"),
            }),
        }
    }

    fn call(&mut self, name: &str, line: usize, col: usize) -> Result<Value, ParseError> {
        let sig = ops::op_by_name(name).ok_or(ParseError {
            line,
            col,
            message: format!("unknown op `{name}`"),
        })?;
        self.next(); // consume `(`

        let mut positional: Vec<Value> = Vec::new();
        let mut keyword: HashMap<String, Value> = HashMap::new();
        let mut seen_keyword = false;

        while self.peek().kind != TokenKind::RParen {
            // keyword?  ident `=` value
            if let TokenKind::Ident(k) = &self.peek().kind {
                if self.toks[self.pos + 1].kind == TokenKind::Eq {
                    let key = k.clone();
                    self.next(); // ident
                    self.next(); // `=`
                    let val = self.arg_value()?;
                    keyword.insert(key, val);
                    seen_keyword = true;
                    self.eat_comma()?;
                    continue;
                }
            }
            if seen_keyword {
                return Err(self.err("positional argument after keyword argument"));
            }
            let val = self.arg_value()?;
            positional.push(val);
            self.eat_comma()?;
        }
        self.next(); // consume `)`

        build(sig, positional, keyword).map_err(|m| ParseError {
            line,
            col,
            message: m,
        })
    }

    /// A single argument: `[ ... ]` list literal or a full expression.
    fn arg_value(&mut self) -> Result<Value, ParseError> {
        if self.peek().kind == TokenKind::LBracket {
            self.next();
            let mut items = Vec::new();
            while self.peek().kind != TokenKind::RBracket {
                items.push(self.expr(0)?);
                if self.peek().kind == TokenKind::Comma {
                    self.next();
                }
            }
            self.next(); // `]`
            return Ok(Value::Array(items));
        }
        self.expr(0)
    }

    fn eat_comma(&mut self) -> Result<(), ParseError> {
        match self.peek().kind {
            TokenKind::Comma => {
                self.next();
                Ok(())
            }
            TokenKind::RParen => Ok(()),
            _ => Err(self.err("expected `,` or `)`")),
        }
    }
}

/// Emit an integral f64 as a JSON integer (so usize fields deserialize), else a float.
fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// A bare number used as an operand becomes a `Const`; objects pass through.
fn promote(v: Value) -> Result<Value, String> {
    match v {
        Value::Number(n) => Ok(json!({ "op": "Const", "value": n })),
        Value::Object(_) => Ok(v),
        Value::Bool(_) => Err("a boolean cannot stand alone as an expression".into()),
        Value::String(_) => Err("a string cannot stand alone as an expression".into()),
        other => Err(format!("invalid expression: {other}")),
    }
}

/// Place bound args into a JSON op object per the signature's field kinds.
fn build(
    sig: &ops::OpSig,
    positional: Vec<Value>,
    mut keyword: HashMap<String, Value>,
) -> Result<Value, String> {
    let mut obj = Map::new();
    obj.insert("op".into(), Value::String(sig.tag.into()));

    if positional.len() > sig.fields.len() {
        return Err(format!(
            "`{}` takes at most {} positional args",
            sig.tag,
            sig.fields.len()
        ));
    }

    // Collect each field's value: positional by index, else keyword by name.
    for (i, field) in sig.fields.iter().enumerate() {
        let name = ops::field_name(field);
        let provided = if i < positional.len() {
            Some(positional[i].clone())
        } else {
            keyword.remove(name)
        };
        let Some(raw) = provided else {
            // required fields must be present
            if matches!(
                field,
                Field::Expr(_) | Field::Num(_) | Field::Str(_) | Field::ExprList(_)
            ) {
                return Err(format!("`{}` requires `{name}`", sig.tag));
            }
            continue;
        };
        obj.insert(name.into(), place(field, raw)?);
    }

    if let Some((k, _)) = keyword.into_iter().next() {
        return Err(format!("`{}` has no field `{k}`", sig.tag));
    }
    Ok(Value::Object(obj))
}

fn place(field: &Field, raw: Value) -> Result<Value, String> {
    match field {
        Field::Expr(_) | Field::ExprOpt(_) => promote(raw),
        Field::ExprList(n) => match raw {
            Value::Array(items) => {
                let mut out = Vec::new();
                for it in items {
                    out.push(promote(it)?);
                }
                Ok(Value::Array(out))
            }
            _ => Err(format!("`{n}` must be a list `[ ... ]`")),
        },
        Field::Num(n) | Field::NumOpt(n) => match raw {
            Value::Number(_) => Ok(raw),
            _ => Err(format!("`{n}` must be a number")),
        },
        Field::BoolOpt(n) => match raw {
            Value::Bool(_) => Ok(raw),
            _ => Err(format!("`{n}` must be true/false")),
        },
        Field::Str(n) | Field::StrOpt(n) => match raw {
            Value::String(_) => Ok(raw),
            _ => Err(format!("`{n}` must be a string")),
        },
        Field::StrListOpt(n) => match raw {
            Value::Array(_) => Ok(raw),
            _ => Err(format!("`{n}` must be a list of strings")),
        },
    }
}

/// (left_bp, right_bp, op_tag) for an infix operator, or None if not infix.
fn infix_binding(op: &str) -> Option<(u8, u8, &'static str)> {
    let level = match op {
        "or" => 1,
        "and" => 2,
        ">" | "<" | ">=" | "<=" => 3,
        "+" | "-" => 4,
        "*" | "/" => 5,
        _ => return None,
    };
    let tag = ops::binop_tag(op)?;
    Some((level * 2, level * 2 + 1, tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(src: &str) -> serde_json::Value {
        parse(src).unwrap()
    }

    #[test]
    fn data_leaf_and_number_call() {
        assert_eq!(
            p("sma(close, 2)"),
            json!({"op":"Average","of":{"op":"Data","name":"close"},"n":2})
        );
    }

    #[test]
    fn keyword_argument() {
        assert_eq!(
            p("rank(close, ascending=false)"),
            json!({"op":"Rank","of":{"op":"Data","name":"close"},"ascending":false})
        );
    }

    #[test]
    fn arity_and_unknown_keyword_errors() {
        assert!(parse("sma(close)").is_err()); // missing n
        assert!(parse("sma(close, 2, 3)").is_err()); // too many
        assert!(parse("sma(close, bogus=2)").is_err()); // unknown keyword
    }

    #[test]
    fn deserializes_into_expr() {
        let v = p("sma(close, 2)");
        let parsed: Result<crate::spec::Expr, _> = serde_json::from_value(v);
        assert!(parsed.is_ok());
    }

    #[test]
    fn comparison_with_const_promotion() {
        assert_eq!(
            p("close > 2"),
            json!({"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":2}})
        );
    }

    #[test]
    fn and_or_precedence() {
        // a and b or c  ==  (a and b) or c
        assert_eq!(
            p("a and b or c"),
            json!({"op":"Or",
                "l":{"op":"And","l":{"op":"Data","name":"a"},"r":{"op":"Data","name":"b"}},
                "r":{"op":"Data","name":"c"}})
        );
    }

    #[test]
    fn arithmetic_precedence_and_unary() {
        // 2 * value + quality  ==  (2*value) + quality ; and  -pe  -> Neg
        assert_eq!(
            p("rank(-pe)"),
            json!({"op":"Rank","of":{"op":"Neg","of":{"op":"Data","name":"pe"}}})
        );
        assert_eq!(
            p("2 * x + y"),
            json!({"op":"Add",
                "l":{"op":"Mul","l":{"op":"Const","value":2},"r":{"op":"Data","name":"x"}},
                "r":{"op":"Data","name":"y"}})
        );
    }

    #[test]
    fn not_binds_between_and_and_comparisons() {
        // not a > b  ==  not (a > b)
        assert_eq!(
            p("not a > b"),
            json!({"op":"Not","of":{"op":"Gt",
                "l":{"op":"Data","name":"a"},"r":{"op":"Data","name":"b"}}})
        );
        // not a and b  ==  (not a) and b
        assert_eq!(
            p("not a and b"),
            json!({"op":"And",
                "l":{"op":"Not","of":{"op":"Data","name":"a"}},
                "r":{"op":"Data","name":"b"}})
        );
        // double negation nests
        assert_eq!(
            p("not not a"),
            json!({"op":"Not","of":{"op":"Not","of":{"op":"Data","name":"a"}}})
        );
        // parenthesized operand
        assert_eq!(
            p("not (a and b)"),
            json!({"op":"Not","of":{"op":"And",
                "l":{"op":"Data","name":"a"},"r":{"op":"Data","name":"b"}}})
        );
    }

    #[test]
    fn let_inlines_subtree() {
        let src = "let ma = sma(close, 2)\nhold_until(entry = close > ma, exit = close < ma, nstocks_limit = 1)";
        assert_eq!(
            p(src),
            json!({
                "op":"HoldUntil",
                "entry":{"op":"Gt","l":{"op":"Data","name":"close"},
                         "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":2}},
                "exit":{"op":"Lt","l":{"op":"Data","name":"close"},
                        "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":2}},
                "nstocks_limit":1
            })
        );
    }

    #[test]
    fn exit_when_and_quantile_row_surface() {
        assert_eq!(
            p("exit_when(close > sma(close, 20), close < sma(close, 60))"),
            json!({
                "op":"ExitWhen",
                "entry":{"op":"Gt","l":{"op":"Data","name":"close"},
                         "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":20}},
                "exit":{"op":"Lt","l":{"op":"Data","name":"close"},
                        "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":60}}
            })
        );
        assert_eq!(
            p("quantile_row(roe, 0.5)"),
            json!({"op":"QuantileRow","of":{"op":"Data","name":"roe"},"c":0.5})
        );
        assert_eq!(
            p("quantile_row(of = pe, c = 0.75)"),
            json!({"op":"QuantileRow","of":{"op":"Data","name":"pe"},"c":0.75})
        );
    }

    #[test]
    fn rebinding_is_an_error() {
        assert!(parse("let a = close\nlet a = pe\na > 1").is_err());
    }

    #[test]
    fn reports_position_on_unclosed_paren() {
        let err = parse("sma(close, 2").unwrap_err();
        assert!(
            err.message.contains("`,` or `)`") || err.message.contains(")"),
            "{}",
            err.message
        );
    }

    #[test]
    fn bare_constant_is_rejected() {
        // Const may not stand alone (engine rule).
        assert!(parse("42").is_err());
    }

    #[test]
    fn unknown_op_names_the_token() {
        let err = parse("frobnicate(close)").unwrap_err();
        assert!(err.message.contains("frobnicate"), "{}", err.message);
    }
}
