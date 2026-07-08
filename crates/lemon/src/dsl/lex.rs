//! Tokenizer for the DSL.

use super::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Num(f64),
    Ident(String),
    Str(String),
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Eq,
    Let,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    let mut col = 1;
    let mut out = Vec::new();

    let bump = |i: &mut usize, col: &mut usize| {
        *i += 1;
        *col += 1;
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => bump(&mut i, &mut col),
            '\n' => {
                i += 1;
                line += 1;
                col = 1;
            }
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '(' => { out.push(Token { kind: TokenKind::LParen, line, col }); bump(&mut i, &mut col); }
            ')' => { out.push(Token { kind: TokenKind::RParen, line, col }); bump(&mut i, &mut col); }
            '[' => { out.push(Token { kind: TokenKind::LBracket, line, col }); bump(&mut i, &mut col); }
            ']' => { out.push(Token { kind: TokenKind::RBracket, line, col }); bump(&mut i, &mut col); }
            ',' => { out.push(Token { kind: TokenKind::Comma, line, col }); bump(&mut i, &mut col); }
            '=' => { out.push(Token { kind: TokenKind::Eq, line, col }); bump(&mut i, &mut col); }
            '>' | '<' => {
                let (start_line, start_col) = (line, col);
                let mut s = c.to_string();
                bump(&mut i, &mut col);
                if i < chars.len() && chars[i] == '=' {
                    s.push('=');
                    bump(&mut i, &mut col);
                }
                out.push(Token { kind: TokenKind::Op(s), line: start_line, col: start_col });
            }
            '+' | '-' | '*' | '/' => {
                out.push(Token { kind: TokenKind::Op(c.to_string()), line, col });
                bump(&mut i, &mut col);
            }
            '"' => {
                let (start_line, start_col) = (line, col);
                bump(&mut i, &mut col);
                let mut s = String::new();
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    bump(&mut i, &mut col);
                }
                if i >= chars.len() {
                    return Err(ParseError { line: start_line, col: start_col, message: "unterminated string".into() });
                }
                bump(&mut i, &mut col); // closing quote
                out.push(Token { kind: TokenKind::Str(s), line: start_line, col: start_col });
            }
            c if c.is_ascii_digit() => {
                let (start_line, start_col) = (line, col);
                let mut s = String::new();
                while i < chars.len() {
                    let d = chars[i];
                    if d.is_ascii_digit() || d == '_' || d == '.' {
                        if d != '_' {
                            s.push(d);
                        }
                        bump(&mut i, &mut col);
                    } else if (d == 'e' || d == 'E')
                        && i + 1 < chars.len()
                        && (chars[i + 1].is_ascii_digit()
                            || ((chars[i + 1] == '+' || chars[i + 1] == '-')
                                && i + 2 < chars.len()
                                && chars[i + 2].is_ascii_digit()))
                    {
                        s.push('e');
                        bump(&mut i, &mut col);
                        if chars[i] == '+' || chars[i] == '-' {
                            s.push(chars[i]);
                            bump(&mut i, &mut col);
                        }
                    } else {
                        break;
                    }
                }
                let value = s.parse::<f64>().map_err(|_| ParseError {
                    line: start_line,
                    col: start_col,
                    message: format!("invalid number `{s}`"),
                })?;
                out.push(Token { kind: TokenKind::Num(value), line: start_line, col: start_col });
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let (start_line, start_col) = (line, col);
                let mut s = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    s.push(chars[i]);
                    bump(&mut i, &mut col);
                }
                let kind = if s == "let" { TokenKind::Let } else { TokenKind::Ident(s) };
                out.push(Token { kind, line: start_line, col: start_col });
            }
            other => {
                return Err(ParseError { line, col, message: format!("unexpected character `{other}`") });
            }
        }
    }

    out.push(Token { kind: TokenKind::Eof, line, col });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_comparison_and_call() {
        use TokenKind::*;
        assert_eq!(
            kinds("close > sma(close, 2)"),
            vec![
                Ident("close".into()), Op(">".into()), Ident("sma".into()),
                LParen, Ident("close".into()), Comma, Num(2.0), RParen, Eof,
            ]
        );
    }

    #[test]
    fn handles_let_comment_underscore_and_scientific() {
        use TokenKind::*;
        let toks = kinds("let x = 1_000_000 # note\nx >= 5e8");
        assert_eq!(toks, vec![
            Let, Ident("x".into()), Eq, Num(1_000_000.0),
            Ident("x".into()), Op(">=".into()), Num(500_000_000.0), Eof,
        ]);
    }

    #[test]
    fn tracks_position_and_rejects_bad_char() {
        let err = lex("a $ b").unwrap_err();
        assert_eq!((err.line, err.col), (1, 3));
    }
}
