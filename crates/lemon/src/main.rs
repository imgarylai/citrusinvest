//! The `lemon` binary: `fmt` (gofmt-style formatter — parses first, so it
//! doubles as a syntax checker) and `lint` (semantic warnings: unknown series,
//! unused `let` bindings). No arg-parsing dep: the surface is small enough for
//! `std::env::args`.
//!
//! ```text
//! lemon fmt  [-w] [file...]                         (no file = read stdin)
//! lemon lint [--series a,b,c] [--series-file p] [file...]
//! ```

use std::io::Read;
use std::process::ExitCode;

/// Reparse + reprint. `path` is only used to label parse errors (gofmt-style).
fn format_source(path: &str, src: &str) -> Result<String, String> {
    match lemon::parse(src) {
        Ok(tree) => Ok(lemon::format(&tree)),
        Err(e) => Err(format!("{path}:{}:{}: {}", e.line, e.col, e.message)),
    }
}

/// Lint one source; returns rendered warnings (empty = clean) or a parse error.
fn lint_source(path: &str, src: &str, series: Option<&[String]>) -> Result<Vec<String>, String> {
    match lemon::lint(src, series) {
        Ok(lints) => Ok(lints
            .iter()
            .map(|l| format!("{path}:{}:{}: warning: {}", l.line, l.col, l.message))
            .collect()),
        Err(e) => Err(format!("{path}:{}:{}: {}", e.line, e.col, e.message)),
    }
}

fn read_stdin() -> Option<String> {
    let mut src = String::new();
    std::io::stdin().read_to_string(&mut src).ok()?;
    Some(src)
}

fn run_fmt(args: Vec<String>) -> ExitCode {
    let mut write = false;
    let mut files: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "-w" | "--write" => write = true,
            _ => files.push(a),
        }
    }

    // No files → stdin to stdout (the `-w` flag is meaningless here, so ignore it).
    if files.is_empty() {
        let Some(src) = read_stdin() else {
            eprintln!("lemon: failed to read stdin");
            return ExitCode::FAILURE;
        };
        return match format_source("<stdin>", &src) {
            Ok(out) => {
                println!("{out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    let mut failed = false;
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("lemon: {path}: {e}");
                failed = true;
                continue;
            }
        };
        match format_source(path, &src) {
            Ok(out) if write => {
                if let Err(e) = std::fs::write(path, format!("{out}\n")) {
                    eprintln!("lemon: {path}: {e}");
                    failed = true;
                }
            }
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_lint(args: Vec<String>) -> ExitCode {
    let mut series: Vec<String> = Vec::new();
    let mut have_series = false;
    let mut files: Vec<String> = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--series" => {
                let Some(list) = it.next() else {
                    eprintln!("lemon: --series needs a comma-separated list");
                    return ExitCode::FAILURE;
                };
                series.extend(list.split(',').map(|s| s.trim().to_string()));
                have_series = true;
            }
            "--series-file" => {
                let Some(path) = it.next() else {
                    eprintln!("lemon: --series-file needs a path");
                    return ExitCode::FAILURE;
                };
                match std::fs::read_to_string(&path) {
                    Ok(body) => {
                        series.extend(
                            body.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .map(String::from),
                        );
                        have_series = true;
                    }
                    Err(e) => {
                        eprintln!("lemon: {path}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            _ => files.push(a),
        }
    }
    let known = have_series.then_some(series.as_slice());

    let mut sources: Vec<(String, String)> = Vec::new();
    if files.is_empty() {
        let Some(src) = read_stdin() else {
            eprintln!("lemon: failed to read stdin");
            return ExitCode::FAILURE;
        };
        sources.push(("<stdin>".into(), src));
    } else {
        for path in files {
            match std::fs::read_to_string(&path) {
                Ok(src) => sources.push((path, src)),
                Err(e) => {
                    eprintln!("lemon: {path}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    let mut dirty = false;
    for (path, src) in &sources {
        match lint_source(path, src, known) {
            Ok(warnings) => {
                for w in &warnings {
                    eprintln!("{w}");
                }
                dirty |= !warnings.is_empty();
            }
            Err(e) => {
                eprintln!("{e}");
                dirty = true;
            }
        }
    }
    if dirty {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("fmt") => run_fmt(args.collect()),
        Some("lint") => run_lint(args.collect()),
        _ => {
            eprintln!(
                "usage: lemon fmt [-w] [file...]\n       lemon lint [--series a,b,c] [--series-file path] [file...]\n       (no file = read stdin)"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_source, lint_source};

    #[test]
    fn formats_valid_and_reports_errors() {
        // Nested call breaks; round-trips through the canonical printer.
        assert_eq!(
            format_source("x.lemon", "rebalance(is_largest(sma(close,5),10))").unwrap(),
            "rebalance(\n  is_largest(\n    sma(close, 5),\n    10\n  )\n)"
        );
        // Parse error is labelled with the file path and 1-based position.
        let err = format_source("x.lemon", "sma(close,").unwrap_err();
        assert!(err.starts_with("x.lemon:"), "got: {err}");
    }

    #[test]
    fn lints_with_and_without_series_list() {
        let series = vec!["close".to_string()];
        let out = lint_source("x.lemon", "clsoe > 1", Some(&series)).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("x.lemon:1:1: warning:"), "{}", out[0]);
        assert!(out[0].contains("did you mean `close`"));
        // no list -> unknown-series check skipped; unused let still reported
        let out = lint_source("x.lemon", "let a = close\nclsoe > 1", None).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("unused let binding `a`"));
    }
}
