//! `lemon fmt` — gofmt-style formatter for the Lemon DSL. Reads `.lemon` source,
//! reparses, and reprints through the one canonical printer so formatted output
//! is stable across every editor and CI. No arg-parsing dep: the surface is just
//! `lemon fmt [-w] [file...]`, so `std::env::args` is enough.

use std::io::Read;
use std::process::ExitCode;

/// Reparse + reprint. `path` is only used to label parse errors (gofmt-style).
fn format_source(path: &str, src: &str) -> Result<String, String> {
    match lemon::parse(src) {
        Ok(tree) => Ok(lemon::format(&tree)),
        Err(e) => Err(format!("{path}:{}:{}: {}", e.line, e.col, e.message)),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("fmt") {
        eprintln!("usage: lemon fmt [-w] [file...]   (no file = read stdin)");
        return ExitCode::FAILURE;
    }

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
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("lemon: failed to read stdin");
            return ExitCode::FAILURE;
        }
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
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

#[cfg(test)]
mod tests {
    use super::format_source;

    #[test]
    fn formats_valid_and_reports_errors() {
        // Nested call breaks; round-trips through the canonical printer.
        assert_eq!(format_source("x.lemon", "rebalance(is_largest(sma(close,5),10))").unwrap(),
            "rebalance(\n  is_largest(\n    sma(close, 5),\n    10\n  )\n)");
        // Parse error is labelled with the file path and 1-based position.
        let err = format_source("x.lemon", "sma(close,").unwrap_err();
        assert!(err.starts_with("x.lemon:"), "got: {err}");
    }
}
