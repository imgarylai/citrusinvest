//! Semantic lints over a parsed strategy. The parser already rejects syntax
//! errors, unknown ops, and bad arities; the two mistakes it *cannot* catch
//! are exactly what this module reports:
//!
//! - **Unknown series** — a typo'd bare identifier silently becomes a `Data`
//!   leaf (`clsoe` parses fine and fails at engine eval). With a known-series
//!   list, the linter flags it at its source position, with a did-you-mean
//!   suggestion.
//! - **Unused `let` bindings** — `let` is parse-time inlining, so a binding
//!   that is never referenced silently vanishes from the tree.

use super::parse::parse_analyzed;
use super::ParseError;

/// One lint warning, at a 1-based source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl std::fmt::Display for Lint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

/// Lint `src`. Parse errors surface as `Err`; a clean parse yields warnings
/// (possibly none). `known_series = None` skips the unknown-series check
/// (unused-`let` is always checked); pass the engine's series list to enable
/// it. Repeated references to the same unknown name warn once.
pub fn lint(src: &str, known_series: Option<&[String]>) -> Result<Vec<Lint>, ParseError> {
    let a = parse_analyzed(src)?;
    let mut out: Vec<Lint> = a
        .unused_lets
        .iter()
        .map(|(name, line, col)| Lint {
            line: *line,
            col: *col,
            message: format!("unused let binding `{name}`"),
        })
        .collect();
    if let Some(known) = known_series {
        let mut seen = std::collections::HashSet::new();
        for (name, line, col) in &a.data_refs {
            if known.iter().any(|k| k == name) || !seen.insert(name.clone()) {
                continue;
            }
            let message = match closest(name, known) {
                Some(s) => format!("unknown series `{name}` — did you mean `{s}`?"),
                None => format!("unknown series `{name}`"),
            };
            out.push(Lint {
                line: *line,
                col: *col,
                message,
            });
        }
    }
    out.sort_by_key(|l| (l.line, l.col));
    Ok(out)
}

/// Closest known name within edit distance 2, if any.
fn closest<'a>(name: &str, known: &'a [String]) -> Option<&'a str> {
    known
        .iter()
        .map(|k| (levenshtein(name, k), k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k.as_str())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_unknown_series_with_suggestion() {
        let known = series(&["close", "pe", "market_cap"]);
        let lints = lint("clsoe > sma(close, 20)", Some(&known)).unwrap();
        assert_eq!(lints.len(), 1);
        assert_eq!((lints[0].line, lints[0].col), (1, 1));
        assert!(
            lints[0].message.contains("did you mean `close`"),
            "{}",
            lints[0].message
        );
        // known names are clean
        assert!(lint("close > sma(close, 20)", Some(&known))
            .unwrap()
            .is_empty());
        // without a series list the check is skipped
        assert!(lint("clsoe > 1", None).unwrap().is_empty());
    }

    #[test]
    fn flags_unused_let_and_dedupes_unknowns() {
        let known = series(&["close"]);
        let src = "let ma = sma(close, 20)\nbogus > 1 and bogus < 2";
        let lints = lint(src, Some(&known)).unwrap();
        // one unused let + ONE unknown-series (bogus referenced twice)
        assert_eq!(lints.len(), 2, "{lints:?}");
        assert!(lints[0].message.contains("unused let binding `ma`"));
        assert!(lints[1].message.contains("unknown series `bogus`"));
        // no suggestion when nothing is close
        assert!(!lints[1].message.contains("did you mean"));
    }

    #[test]
    fn parse_errors_pass_through() {
        assert!(lint("sma(close,", None).is_err());
    }
}
