//! Editor language services for the Lemon DSL — pure, I/O-free, editor-agnostic.
//!
//! This module turns the lexer + op catalog (the same single source of truth the
//! parser and schema generator consume) into the three primitives every code
//! editor needs:
//!
//! - [`diagnostics`] — parse/lex errors and the semantic lints from
//!   [`crate::lint`] (unused `let`s; unknown series when a series list is given),
//!   as ranged [`Diagnostic`]s. The linter is the single diagnostics source, so
//!   the editor squiggles match `lemon lint` exactly.
//! - [`hover`] — the signature and description of the op / operator / series
//!   under the cursor, rendered as Markdown.
//! - [`completions`] — op names, keyword-argument names for the enclosing call,
//!   `let`-bound names in scope, known series, and keyword literals.
//!
//! Everything here is a pure function of `(source, position)`. The same core
//! backs both the WASM boundary (`lemon-wasm`, for the in-browser editor) and
//! the native language server (`lemon-lsp`, over `tower-lsp`), so hover text and
//! completions can never drift between the two surfaces.
//!
//! # Positions
//!
//! All positions are **1-based** `(line, col)`, matching [`crate::ParseError`]
//! and the lexer: `col` is the column of a character, and a cursor "after" the
//! last typed character sits at `len + 1`. Ranges are half-open — `end_col` is
//! one past the last covered column. Editors that speak 0-based positions (LSP)
//! convert at their boundary.

use crate::dsl::lex::{lex, Token, TokenKind};
use crate::meta::{function_ops, OpInfo};

/// Diagnostic severity. Parse/lex errors are [`Severity::Error`]; semantic lints
/// from [`crate::lint`] are [`Severity::Warning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A ranged diagnostic: `[ (line, col) .. (end_line, end_col) )`, 1-based, with
/// `end_col` exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub severity: Severity,
    pub message: String,
}

/// What a [`CompletionItem`] refers to — lets the editor pick an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    /// A callable op (`sma`, `rank`, …).
    Function,
    /// A keyword-argument name for the enclosing call (`ascending`).
    Field,
    /// A `let`-bound name in scope.
    Variable,
    /// A known input data series (`close`, `pe`, …).
    Series,
    /// A language keyword / literal (`let`, `and`, `true`, …).
    Keyword,
}

impl CompletionKind {
    /// Lowercase tag used in the JSON boundary.
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionKind::Function => "function",
            CompletionKind::Field => "field",
            CompletionKind::Variable => "variable",
            CompletionKind::Series => "series",
            CompletionKind::Keyword => "keyword",
        }
    }
}

/// One completion candidate. `insert_text` is the text to insert (equal to
/// `label` for everything except keyword args, which insert `name=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// A short, one-line signature or category (shown to the right of the label).
    pub detail: String,
    /// Longer Markdown documentation (the op description, for functions).
    pub documentation: String,
    pub insert_text: String,
}

/// The hover card for the token under the cursor: a range to highlight plus the
/// Markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub markdown: String,
}

/// Input series the engine always provides from price data. Fundamentals are
/// open-ended (any identifier is accepted), so this list drives completion only.
pub const PRICE_SERIES: &[&str] = &["open", "high", "low", "close", "volume"];

/// A curated set of common fundamental series, offered as completions purely as
/// a convenience. Not exhaustive and not authoritative — the engine's data
/// context is the source of truth.
pub const COMMON_FUNDAMENTALS: &[&str] = &[
    "pe",
    "pb",
    "ps",
    "roe",
    "roa",
    "eps",
    "market_cap",
    "revenue_growth",
    "dividend_yield",
];

/// Reserved words that are never data series or op calls.
const KEYWORDS: &[&str] = &["let", "and", "or", "true", "false"];

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

/// Compute diagnostics for `src`: the (single) parse or lex error if the source
/// is invalid, otherwise the semantic lints from [`crate::lint`] as ranged
/// warnings.
///
/// `known_series` is the engine's list of valid data-series names. Pass it to
/// enable the unknown-series check (typo'd `Data` leaves, with did-you-mean
/// suggestions); pass `None` to skip it (unused-`let` warnings still fire). The
/// LSP/editor has no series list unless configured, so it defaults to `None`.
pub fn diagnostics(src: &str, known_series: Option<&[String]>) -> Vec<Diagnostic> {
    // A lex failure is a hard stop — there are no tokens to lint or span.
    let toks = match lex(src) {
        Ok(t) => t,
        Err(e) => {
            return vec![Diagnostic {
                line: e.line,
                col: e.col,
                end_line: e.line,
                end_col: e.col + 1,
                severity: Severity::Error,
                message: e.message,
            }]
        }
    };

    // The linter is the diagnostics source: it returns the parse error (if any)
    // or the semantic lints for a clean parse.
    match crate::lint(src, known_series) {
        Err(e) => {
            // A parse failure spans the token at its position when one exists, so
            // the squiggle covers the whole word rather than a single column.
            let (end_line, end_col) = token_at(&toks, e.line, e.col)
                .map(token_end)
                .unwrap_or((e.line, e.col + 1));
            vec![Diagnostic {
                line: e.line,
                col: e.col,
                end_line,
                end_col,
                severity: Severity::Error,
                message: e.message,
            }]
        }
        Ok(lints) => lints
            .into_iter()
            .map(|l| {
                let (end_line, end_col) = token_at(&toks, l.line, l.col)
                    .map(token_end)
                    .unwrap_or((l.line, l.col + 1));
                Diagnostic {
                    line: l.line,
                    col: l.col,
                    end_line,
                    end_col,
                    severity: Severity::Warning,
                    message: l.message,
                }
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

/// Markdown hover for the token at 1-based `(line, col)`, or `None` when there is
/// nothing documented there (whitespace, punctuation, numbers, unknown series).
pub fn hover(src: &str, line: usize, col: usize) -> Option<HoverInfo> {
    let toks = lex(src).ok()?;
    let t = token_at(&toks, line, col)?;
    let (end_line, end_col) = token_end(t);

    let markdown = match &t.kind {
        TokenKind::Ident(name) => {
            if let Some(op) = lookup_op(name) {
                op_markdown(&op)
            } else if let Some(kw) = keyword_markdown(name) {
                kw
            } else if PRICE_SERIES.contains(&name.as_str()) {
                format!("**`{name}`** — input data series (price).")
            } else {
                // Any other identifier is a data-series reference.
                format!("**`{name}`** — data series reference.")
            }
        }
        TokenKind::Op(sym) => binop_markdown(sym)?,
        TokenKind::Let => keyword_markdown("let")?,
        _ => return None,
    };

    Some(HoverInfo {
        line: t.line,
        col: t.col,
        end_line,
        end_col,
        markdown,
    })
}

/// The full hover body for a callable op: its signature, description, and aliases.
fn op_markdown(op: &OpInfo) -> String {
    let mut s = format!("```lemon\n{}\n```\n\n{}", signature(op), op.description);
    if !op.aliases.is_empty() {
        let aliases = op
            .aliases
            .iter()
            .map(|a| format!("`{a}`"))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("\n\n*Aliases: {aliases}*"));
    }
    s
}

/// A one-line signature: `sma(of, n)`, with optional fields marked `?` and a
/// trailing `= default` where the catalog records one.
fn signature(op: &OpInfo) -> String {
    let params: Vec<String> = op
        .fields
        .iter()
        .map(|f| {
            let opt = if f.required { "" } else { "?" };
            match &f.default {
                Some(d) => format!("{}{opt}={d}", f.name),
                None => format!("{}{opt}", f.name),
            }
        })
        .collect();
    format!("{}({})", op.name, params.join(", "))
}

fn keyword_markdown(word: &str) -> Option<String> {
    let body = match word {
        "let" => "**`let`** — bind a name to a sub-expression: `let ma = sma(close, 20)`. Bindings are inlined at parse time.",
        "and" => "**`and`** — logical AND. Yields `1.0` where both operands are truthy, else `0.0`.",
        "or" => "**`or`** — logical OR. Yields `1.0` where either operand is truthy, else `0.0`.",
        "true" => "**`true`** — boolean literal (keyword-argument values only).",
        "false" => "**`false`** — boolean literal (keyword-argument values only).",
        _ => return None,
    };
    Some(body.to_string())
}

fn binop_markdown(sym: &str) -> Option<String> {
    let desc = match sym {
        ">" => "greater-than",
        "<" => "less-than",
        ">=" => "greater-than-or-equal",
        "<=" => "less-than-or-equal",
        "+" => "addition",
        "-" => "subtraction (or unary negation)",
        "*" => "multiplication",
        "/" => "division",
        _ => return None,
    };
    Some(format!(
        "**`{sym}`** — {desc}. Comparisons output `1.0`/`0.0`."
    ))
}

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// Completion candidates for the cursor at 1-based `(line, col)`.
///
/// The list is filtered by the identifier prefix under the cursor and ordered by
/// relevance: keyword arguments for the enclosing call first (when inside a
/// call), then `let`-bound names, ops, series, and keyword literals.
pub fn completions(src: &str, line: usize, col: usize) -> Vec<CompletionItem> {
    let toks = match lex(src) {
        Ok(t) => t,
        // Even on a lex error, offer the static vocabulary so completion still
        // works while the user is mid-edit.
        Err(_) => return filter_items(static_items(&[]), ""),
    };

    let prefix = prefix_at(&toks, line, col);
    let before = tokens_before(&toks, line, col);
    let enclosing = enclosing_call(&before);
    let bound = let_bound_names(&before);

    let mut items = Vec::new();

    // Keyword-argument names for the call we're inside, most relevant first.
    if let Some(op) = enclosing.as_deref().and_then(lookup_op) {
        for f in &op.fields {
            items.push(CompletionItem {
                label: f.name.to_string(),
                kind: CompletionKind::Field,
                detail: format!("{} argument ({})", op.name, f.kind),
                documentation: String::new(),
                insert_text: format!("{}=", f.name),
            });
        }
    }

    // `let`-bound names visible at the cursor.
    for name in bound {
        items.push(CompletionItem {
            label: name.clone(),
            kind: CompletionKind::Variable,
            detail: "let-bound".to_string(),
            documentation: String::new(),
            insert_text: name,
        });
    }

    items.extend(static_items(&[]));
    filter_items(items, &prefix)
}

/// The vocabulary that is always available regardless of context: every op, the
/// known series, and the keyword literals. `_extra` is reserved for future
/// context-specific additions.
fn static_items(_extra: &[&str]) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    for op in function_ops() {
        items.push(CompletionItem {
            label: op.name.to_string(),
            kind: CompletionKind::Function,
            detail: signature(&op),
            documentation: op.description.to_string(),
            insert_text: op.name.to_string(),
        });
    }

    for &s in PRICE_SERIES {
        items.push(series_item(s, "price series"));
    }
    for &s in COMMON_FUNDAMENTALS {
        items.push(series_item(s, "fundamental series"));
    }
    for &kw in KEYWORDS {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
            detail: "keyword".to_string(),
            documentation: String::new(),
            insert_text: kw.to_string(),
        });
    }
    items
}

fn series_item(name: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: name.to_string(),
        kind: CompletionKind::Series,
        detail: detail.to_string(),
        documentation: String::new(),
        insert_text: name.to_string(),
    }
}

/// Keep only items whose label starts with `prefix` (case-insensitive). An empty
/// prefix keeps everything. De-duplicates by `(label, kind)` so a series that is
/// also offered as a keyword argument does not appear twice for the same reason.
fn filter_items(items: Vec<CompletionItem>, prefix: &str) -> Vec<CompletionItem> {
    let pfx = prefix.to_ascii_lowercase();
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|it| it.label.to_ascii_lowercase().starts_with(&pfx))
        .filter(|it| seen.insert((it.label.clone(), it.kind)))
        .collect()
}

// ---------------------------------------------------------------------------
// Shared token helpers
// ---------------------------------------------------------------------------

/// Look up a callable op by its canonical name or any alias.
fn lookup_op(name: &str) -> Option<OpInfo> {
    function_ops()
        .into_iter()
        .find(|o| o.name == name || o.aliases.contains(&name))
}

/// The `let`-bound names introduced anywhere in `toks` (the token after each
/// `let`). Used both to seed completion and to exclude bindings from the typo
/// lint.
fn let_bound_names(toks: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if t.kind == TokenKind::Let {
            if let Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) = toks.get(i + 1)
            {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
        }
    }
    out
}

/// The op name of the innermost call whose argument list the cursor sits in, if
/// any. Walks the bracket stack: `name(` opens a call frame, `(`/`[` open a
/// non-call frame, and each closer pops. `toks` must already be truncated to the
/// tokens before the cursor (see [`tokens_before`]).
fn enclosing_call(toks: &[Token]) -> Option<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match &t.kind {
            TokenKind::LParen => {
                let name = match i.checked_sub(1).and_then(|p| toks.get(p)) {
                    Some(Token {
                        kind: TokenKind::Ident(n),
                        ..
                    }) => Some(n.clone()),
                    _ => None,
                };
                stack.push(name);
            }
            TokenKind::LBracket => stack.push(None),
            TokenKind::RParen | TokenKind::RBracket => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.into_iter().rev().flatten().next()
}

/// The identifier prefix the cursor is currently inside/just after, or `""`.
fn prefix_at(toks: &[Token], line: usize, col: usize) -> String {
    for t in toks {
        if let TokenKind::Ident(name) = &t.kind {
            let len = name.chars().count();
            if t.line == line && t.col <= col && col <= t.col + len {
                let take = col - t.col;
                return name.chars().take(take).collect();
            }
        }
    }
    String::new()
}

/// The tokens strictly before the cursor, in order (dropping the `Eof`). A token
/// counts as "before" when it starts before the cursor position.
fn tokens_before(toks: &[Token], line: usize, col: usize) -> Vec<Token> {
    toks.iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .filter(|t| t.line < line || (t.line == line && t.col < col))
        .cloned()
        .collect()
}

/// The token whose span covers 1-based `(line, col)`, if any. `Eof` never
/// matches. The cursor is treated as covered when `col` is within `[start, end)`.
fn token_at(toks: &[Token], line: usize, col: usize) -> Option<&Token> {
    toks.iter().find(|t| {
        if t.kind == TokenKind::Eof || t.line != line {
            return false;
        }
        let (_, end_col) = token_end(t);
        t.col <= col && col < end_col
    })
}

/// The exclusive end `(line, col)` of a token's source span. Multi-line tokens do
/// not occur in this grammar, so the end line always equals the start line.
fn token_end(t: &Token) -> (usize, usize) {
    let len = match &t.kind {
        TokenKind::Ident(s) | TokenKind::Op(s) => s.chars().count(),
        TokenKind::Str(s) => s.chars().count() + 2, // surrounding quotes
        TokenKind::Num(_) => 1,                     // width unknown post-lex; a single-column caret
        TokenKind::Let => 3,
        TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::Comma
        | TokenKind::Eq => 1,
        TokenKind::Eof => 0,
    };
    (t.line, t.col + len)
}

// ---------------------------------------------------------------------------
// Semantic tokens (syntax highlighting)
// ---------------------------------------------------------------------------

/// A syntax-highlight classification for a run of source. This is the single
/// source of truth for lemon highlighting: every editor surface (the in-browser
/// CodeMirror playground, `citrus-fund`, an LSP semantic-tokens provider) colours
/// from [`tokens`], so highlighting can never drift from the lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Comment,
    Number,
    Str,
    Keyword,
    Function,
    Parameter,
    Series,
    Operator,
    Punctuation,
}

impl TokenType {
    /// Lowercase tag used in the JSON boundary and as an editor theme key.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenType::Comment => "comment",
            TokenType::Number => "number",
            TokenType::Str => "string",
            TokenType::Keyword => "keyword",
            TokenType::Function => "function",
            TokenType::Parameter => "parameter",
            TokenType::Series => "series",
            TokenType::Operator => "operator",
            TokenType::Punctuation => "punctuation",
        }
    }
}

/// A classified source span, `[ (line, col) .. (end_line, end_col) )`, 1-based
/// with `end_col` exclusive — same convention as [`Diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub token_type: TokenType,
}

/// Classify every token in `src` for syntax highlighting.
///
/// Built on the parser's own [`lex`], so token boundaries and kinds match the
/// language exactly. Identifiers are classified structurally — `name(` is a
/// function call, `name=` a keyword argument, `and`/`or`/`not`/`true`/`false`
/// are keywords, and anything else is a series reference — so no op list is
/// hard-coded and nothing drifts when ops are added. The only rules re-derived
/// here are the ones the lexer does not preserve: numeric span *width* (the
/// parsed `f64` loses the original spelling like `1_000`/`5e8`) and line
/// comments (which the lexer discards). Returns tokens in source order; on a lex
/// error (e.g. an unterminated string mid-edit) returns just the comment spans.
pub fn tokens(src: &str) -> Vec<SemanticToken> {
    let lines: Vec<Vec<char>> = src.split('\n').map(|l| l.chars().collect()).collect();
    let mut out = comment_spans(&lines);

    if let Ok(toks) = lex(src) {
        for (i, t) in toks.iter().enumerate() {
            let token_type = match &t.kind {
                TokenKind::Eof => continue,
                TokenKind::Num(_) => TokenType::Number,
                TokenKind::Str(_) => TokenType::Str,
                TokenKind::Let => TokenType::Keyword,
                TokenKind::Op(_) | TokenKind::Eq => TokenType::Operator,
                TokenKind::LParen
                | TokenKind::RParen
                | TokenKind::LBracket
                | TokenKind::RBracket
                | TokenKind::Comma => TokenType::Punctuation,
                TokenKind::Ident(s) => classify_ident(s, toks.get(i + 1)),
            };
            // Reuse `token_end` for every kind whose width the lexer preserves;
            // numbers are the one exception, measured back off the source.
            let (end_line, end_col) = match &t.kind {
                TokenKind::Num(_) => (t.line, t.col + number_len(&lines, t.line, t.col)),
                _ => token_end(t),
            };
            out.push(SemanticToken {
                line: t.line,
                col: t.col,
                end_line,
                end_col,
                token_type,
            });
        }
    }

    out.sort_by_key(|t| (t.line, t.col));
    out
}

/// Structural classification of a bareword: reserved words first, then by what
/// follows — `name(` → call, `name=` → keyword argument, otherwise a series.
fn classify_ident(s: &str, next: Option<&Token>) -> TokenType {
    if matches!(s, "and" | "or" | "not" | "true" | "false") {
        return TokenType::Keyword;
    }
    match next.map(|t| &t.kind) {
        Some(TokenKind::LParen) => TokenType::Function,
        Some(TokenKind::Eq) => TokenType::Parameter,
        _ => TokenType::Series,
    }
}

/// Character width of the numeric literal starting at 1-based `(line, col)`,
/// mirroring the digit / `_` / `.` / exponent run the lexer accepts.
fn number_len(lines: &[Vec<char>], line: usize, col: usize) -> usize {
    let Some(row) = lines.get(line - 1) else {
        return 1;
    };
    let start = col - 1;
    let mut j = start;
    while j < row.len() {
        let d = row[j];
        if d.is_ascii_digit() || d == '_' || d == '.' {
            j += 1;
        } else if (d == 'e' || d == 'E')
            && j + 1 < row.len()
            && (row[j + 1].is_ascii_digit()
                || ((row[j + 1] == '+' || row[j + 1] == '-')
                    && j + 2 < row.len()
                    && row[j + 2].is_ascii_digit()))
        {
            j += 2; // the `e` and the sign or first exponent digit
        } else {
            break;
        }
    }
    (j - start).max(1)
}

/// Line-comment spans (`# … EOL`) that are not inside a string literal, mirroring
/// the lexer: `#` outside a string starts a comment, and `"` toggles string
/// state (lemon strings have no escapes). `in_string` intentionally persists
/// across lines to match the lexer's newline-tolerant string scan.
fn comment_spans(lines: &[Vec<char>]) -> Vec<SemanticToken> {
    let mut out = Vec::new();
    let mut in_string = false;
    for (li, row) in lines.iter().enumerate() {
        let mut c = 0;
        while c < row.len() {
            match row[c] {
                '"' => in_string = !in_string,
                '#' if !in_string => {
                    out.push(SemanticToken {
                        line: li + 1,
                        col: c + 1,
                        end_line: li + 1,
                        end_col: row.len() + 1,
                        token_type: TokenType::Comment,
                    });
                    break; // the rest of the line is the comment
                }
                _ => {}
            }
            c += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- diagnostics --------------------------------------------------------

    fn series(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn clean_source_has_no_diagnostics() {
        assert!(diagnostics("close > sma(close, 2)", None).is_empty());
        // With a series list, valid series stay clean too.
        assert!(diagnostics("close > sma(close, 2)", Some(&series(&["close"]))).is_empty());
    }

    #[test]
    fn parse_error_becomes_a_ranged_error_diagnostic() {
        let diags = diagnostics("sma(close, 2", None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].end_col > diags[0].col);
    }

    #[test]
    fn lex_error_is_reported_without_panicking() {
        let diags = diagnostics("close $ 1", None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!((diags[0].line, diags[0].col), (1, 7));
    }

    #[test]
    fn parse_error_on_unknown_op_spans_the_word() {
        let diags = diagnostics("frobnicate(close)", None);
        assert_eq!(diags.len(), 1);
        // The squiggle covers the whole `frobnicate` token, not one column.
        assert_eq!(diags[0].col, 1);
        assert_eq!(diags[0].end_col, 1 + "frobnicate".len());
    }

    #[test]
    fn unknown_series_warning_is_ranged_when_a_series_list_is_given() {
        let diags = diagnostics("clsoe > 1", Some(&series(&["close", "pe"])));
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("close"), "{}", diags[0].message);
        // The range spans the whole misspelled token `clsoe`.
        assert_eq!((diags[0].col, diags[0].end_col), (1, 1 + "clsoe".len()));
    }

    #[test]
    fn no_unknown_series_check_without_a_list() {
        // A typo'd series is silent without the engine's series list.
        assert!(diagnostics("clsoe > 1", None).is_empty());
    }

    #[test]
    fn unused_let_binding_warns_even_without_a_series_list() {
        let diags = diagnostics("let ma = sma(close, 20)\nclose > 1", None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("unused let binding `ma`"));
        // The range spans the binding name `ma`.
        assert_eq!(diags[0].end_col, diags[0].col + "ma".len());
    }

    // --- hover --------------------------------------------------------------

    #[test]
    fn hover_on_op_shows_signature_and_description() {
        let h = hover("close > sma(close, 2)", 1, 9).expect("hover on sma");
        assert!(h.markdown.contains("sma(of, n)"), "{}", h.markdown);
        assert!(h.markdown.contains("moving average"));
        assert!(h.markdown.contains("Aliases"), "{}", h.markdown);
        // Range covers the three-character `sma` token.
        assert_eq!((h.line, h.col), (1, 9));
        assert_eq!(h.end_col, 12);
    }

    #[test]
    fn hover_on_operator_and_series_and_keyword() {
        assert!(hover("close > 1", 1, 7)
            .unwrap()
            .markdown
            .contains("greater"));
        assert!(hover("close > 1", 1, 1).unwrap().markdown.contains("price"));
        assert!(hover("let a = close\na > 1", 1, 1)
            .unwrap()
            .markdown
            .contains("bind"));
    }

    #[test]
    fn hover_on_unknown_series_and_nothing_are_distinguished() {
        assert!(hover("roic > 1", 1, 1)
            .unwrap()
            .markdown
            .contains("data series"));
        // Whitespace / punctuation / numbers → no hover.
        assert!(hover("close > 1", 1, 6).is_none()); // the space
        assert!(hover("close > 1", 1, 9).is_none()); // the number `1`
        assert!(hover("", 1, 1).is_none());
    }

    #[test]
    fn hover_ignores_lex_errors() {
        assert!(hover("close $", 1, 1).is_none());
    }

    #[test]
    fn hover_on_op_without_aliases_omits_alias_line() {
        // `ema` has no aliases — the alias line must be absent.
        let h = hover("ema(close, 5)", 1, 1).unwrap();
        assert!(h.markdown.contains("ema(of, n)"));
        assert!(!h.markdown.contains("Aliases"), "{}", h.markdown);
    }

    #[test]
    fn hover_on_boolean_literal_and_logical_words() {
        assert!(hover("rank(close, ascending=true)", 1, 23)
            .unwrap()
            .markdown
            .contains("boolean"));
        assert!(hover("a and b", 1, 3).unwrap().markdown.contains("AND"));
        assert!(hover("a or b", 1, 3).unwrap().markdown.contains("OR"));
    }

    #[test]
    fn every_operator_symbol_has_hover_text() {
        for sym in [">", "<", ">=", "<=", "+", "-", "*", "/"] {
            assert!(binop_markdown(sym).is_some(), "no hover for `{sym}`");
        }
        assert!(binop_markdown("??").is_none());
    }

    #[test]
    fn every_keyword_has_hover_text() {
        for kw in ["let", "and", "or", "true", "false"] {
            assert!(keyword_markdown(kw).is_some(), "no hover for `{kw}`");
        }
        assert!(keyword_markdown("close").is_none());
    }

    // --- completions --------------------------------------------------------

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn completes_op_names_by_prefix() {
        let items = completions("sm", 1, 3);
        let ls = labels(&items);
        assert!(ls.contains(&"sma"));
        assert!(!ls.contains(&"rank"), "prefix `sm` should exclude rank");
    }

    #[test]
    fn empty_prefix_offers_the_whole_vocabulary() {
        let items = completions("", 1, 1);
        let ls = labels(&items);
        assert!(ls.contains(&"sma"));
        assert!(ls.contains(&"close"));
        assert!(ls.contains(&"let"));
    }

    #[test]
    fn inside_a_call_offers_keyword_arguments_first() {
        // Cursor inside rank(...) — `ascending`/`pct` should be offered as fields.
        let items = completions("rank(close, )", 1, 13);
        let field = items
            .iter()
            .find(|i| i.label == "ascending")
            .expect("ascending field offered");
        assert_eq!(field.kind, CompletionKind::Field);
        assert_eq!(field.insert_text, "ascending=");
    }

    #[test]
    fn completes_let_bound_names() {
        let src = "let ma = sma(close, 20)\nclose > m";
        let items = completions(src, 2, 10);
        let ma = items.iter().find(|i| i.label == "ma").expect("ma offered");
        assert_eq!(ma.kind, CompletionKind::Variable);
    }

    #[test]
    fn enclosing_call_handles_lists_closed_calls_and_leading_paren() {
        // Inside a list literal nested in a call → still offers the call's fields.
        let items = completions("neutralize(close, [pe, ", 1, 23);
        assert!(items
            .iter()
            .any(|i| i.label == "by" && i.kind == CompletionKind::Field));

        // After a fully-closed call → no enclosing call, just the vocabulary.
        let items = completions("sma(close, 2) and cl", 1, 21);
        assert!(labels(&items).contains(&"close"));
        assert!(!items.iter().any(|i| i.kind == CompletionKind::Field));

        // A leading `(` with no op before it must not panic (checked_sub).
        let items = completions("(cl", 1, 4);
        assert!(labels(&items).contains(&"close"));
    }

    #[test]
    fn completion_survives_a_lex_error() {
        // A stray `$` makes lexing fail; static vocabulary is still returned.
        let items = completions("$sm", 1, 1);
        assert!(!items.is_empty());
        assert!(labels(&items).contains(&"sma"));
    }

    #[test]
    fn no_duplicate_labels_of_the_same_kind() {
        let items = completions("", 1, 1);
        let mut seen = std::collections::HashSet::new();
        for it in &items {
            assert!(
                seen.insert((it.label.clone(), it.kind)),
                "duplicate: {} / {:?}",
                it.label,
                it.kind
            );
        }
    }

    // --- helpers ------------------------------------------------------------

    #[test]
    fn token_end_covers_every_kind_and_let_without_name() {
        let toks = lex("let x = \"s\" >= 1 + (a) [ , ]").unwrap();
        for t in &toks {
            let (_, end) = token_end(t);
            assert!(end >= t.col);
        }
        // A string span includes both surrounding quotes: `"s"` is three columns.
        let str_tok = toks
            .iter()
            .find(|t| matches!(t.kind, TokenKind::Str(_)))
            .unwrap();
        assert_eq!(token_end(str_tok), (str_tok.line, str_tok.col + 3));
        // Eof has zero width.
        let eof = toks.last().unwrap();
        assert_eq!(token_end(eof), (eof.line, eof.col));
        // `let` with no following identifier introduces no binding and never panics.
        assert!(let_bound_names(&lex("let").unwrap()).is_empty());
    }

    #[test]
    fn completion_kind_tags_round_trip() {
        assert_eq!(CompletionKind::Function.as_str(), "function");
        assert_eq!(CompletionKind::Field.as_str(), "field");
        assert_eq!(CompletionKind::Variable.as_str(), "variable");
        assert_eq!(CompletionKind::Series.as_str(), "series");
        assert_eq!(CompletionKind::Keyword.as_str(), "keyword");
    }

    // --- semantic tokens (highlighting) -------------------------------------

    fn typed(src: &str) -> Vec<(&'static str, usize, usize, usize)> {
        tokens(src)
            .into_iter()
            .map(|t| (t.token_type.as_str(), t.line, t.col, t.end_col))
            .collect()
    }

    #[test]
    fn classifies_call_series_number_and_punctuation() {
        // is_largest( -> function; sma( -> function; close -> series; 2/3 -> number
        assert_eq!(
            typed("is_largest(sma(close, 2), 3)"),
            vec![
                ("function", 1, 1, 11),     // is_largest
                ("punctuation", 1, 11, 12), // (
                ("function", 1, 12, 15),    // sma
                ("punctuation", 1, 15, 16), // (
                ("series", 1, 16, 21),      // close
                ("punctuation", 1, 21, 22), // ,
                ("number", 1, 23, 24),      // 2
                ("punctuation", 1, 24, 25), // )
                ("punctuation", 1, 25, 26), // ,
                ("number", 1, 27, 28),      // 3
                ("punctuation", 1, 28, 29), // )
            ]
        );
    }

    #[test]
    fn keyword_args_logic_and_operators() {
        // ascending= -> parameter; true -> keyword; and/or -> keyword; > -> operator
        let out = typed("rank(close, ascending=true) and close > sma(close, 2)");
        assert!(out.contains(&("parameter", 1, 13, 22))); // ascending
        assert!(out.contains(&("operator", 1, 22, 23))); // =
        assert!(out.contains(&("keyword", 1, 23, 27))); // true
        assert!(out.contains(&("keyword", 1, 29, 32))); // and
        assert!(out.contains(&("operator", 1, 39, 40))); // >
    }

    #[test]
    fn not_is_keyword_even_before_paren() {
        // `not` must stay a keyword, not be misread as a call by the `(` lookahead.
        let out = typed("not (close)");
        assert_eq!(out[0], ("keyword", 1, 1, 4));
    }

    #[test]
    fn numbers_keep_their_full_width() {
        // Underscores and exponents are lost in the parsed f64; width is measured
        // back off the source so the whole literal is covered.
        assert_eq!(
            typed("x >= 1_000_000"),
            vec![
                ("series", 1, 1, 2),   // x
                ("operator", 1, 3, 5), // >=
                ("number", 1, 6, 15),  // 1_000_000 (9 chars)
            ]
        );
        assert!(typed("5e8").contains(&("number", 1, 1, 4)));
    }

    #[test]
    fn comments_are_spans_and_ignore_hashes_in_strings() {
        // A trailing comment is highlighted; a `#` inside a string is not.
        let out = typed("close # buy\n");
        assert!(out.contains(&("comment", 1, 7, 12)));
        let s = typed("in_sector(close, \"A#B\")");
        assert!(s.iter().all(|t| t.0 != "comment"));
    }

    #[test]
    fn lex_error_still_yields_comment_spans() {
        // Unterminated string is a lex error; comments before it survive.
        let out = typed("# note\nsma(close, \"oops");
        assert_eq!(out.first(), Some(&("comment", 1, 1, 7)));
    }

    #[test]
    fn token_type_tags_round_trip() {
        for (ty, tag) in [
            (TokenType::Comment, "comment"),
            (TokenType::Number, "number"),
            (TokenType::Str, "string"),
            (TokenType::Keyword, "keyword"),
            (TokenType::Function, "function"),
            (TokenType::Parameter, "parameter"),
            (TokenType::Series, "series"),
            (TokenType::Operator, "operator"),
            (TokenType::Punctuation, "punctuation"),
        ] {
            assert_eq!(ty.as_str(), tag);
        }
    }
}
