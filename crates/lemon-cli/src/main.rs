//! The `lemon` binary: `fmt` (gofmt-style formatter — parses first, so it
//! doubles as a syntax checker), `lint` (semantic warnings: unknown series,
//! unused `let` bindings), `check` (validate strategy envelopes), and `run`
//! (lower a strategy and backtest it over a local data-layout tree). No
//! arg-parsing dep: the surface is small enough for `std::env::args`.
//!
//! ```text
//! lemon fmt   [-w] [file...]                        (no file = read stdin)
//! lemon lint  [--series a,b,c] [--series-file p] [file...]
//! lemon check [file...]                             (validate strategy envelopes)
//! lemon run   --data <dir> [--from N] [--to N] [--fee-ratio F]
//!             [--slippage-ratio F] [--price-key K] [--benchmark SYM]
//!             [--out path] [file]
//! ```
//!
//! The language crate (`lemon-lang`) stays pure and I/O-free — `yuzu-core`
//! depends on it, so the engine wiring for `run` has to live out here.

use std::io::Read;
use std::path::PathBuf;
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

/// Validate one strategy-envelope document; returns rendered errors (empty = ok).
fn check_source(path: &str, src: &str) -> Result<(), Vec<String>> {
    lemon::envelope::check(src)
        .map(|_| ())
        .map_err(|errs| errs.into_iter().map(|e| format!("{path}: {e}")).collect())
}

fn run_check(args: Vec<String>) -> ExitCode {
    let files: Vec<String> = args;

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

    let mut failed = false;
    for (path, src) in &sources {
        match check_source(path, src) {
            Ok(()) => println!("{path}: ok"),
            Err(errs) => {
                for e in &errs {
                    eprintln!("{e}");
                }
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

/// Arguments for `lemon run`. Flag names mirror `yuzu-cli run` so the two
/// runners stay interchangeable; this is the subset that covers a plain
/// single-strategy run (the full stop/delist/bootstrap knob set stays on
/// `yuzu-cli` until the envelope carries config — see docs/strategy-envelope.md).
#[derive(Debug)]
struct RunArgs {
    /// Strategy source path; `None` = stdin.
    file: Option<String>,
    /// Data-layout tree root (docs/data-layout.md).
    data: PathBuf,
    from: i32,
    to: i32,
    fee_ratio: f64,
    slippage_ratio: f64,
    price_key: String,
    benchmark: Option<String>,
    /// Report destination; `None` = stdout.
    out: Option<PathBuf>,
}

fn parse_run_args(args: Vec<String>) -> Result<RunArgs, String> {
    fn value(flag: &str, v: Option<String>) -> Result<String, String> {
        v.ok_or_else(|| format!("{flag} needs a value"))
    }
    fn num<T: std::str::FromStr>(flag: &str, v: Option<String>) -> Result<T, String> {
        let v = value(flag, v)?;
        v.parse()
            .map_err(|_| format!("{flag}: invalid value `{v}`"))
    }

    let mut file: Option<String> = None;
    let mut data: Option<PathBuf> = None;
    let mut from: i32 = 20000101;
    let mut to: i32 = 99991231;
    let mut fee_ratio: f64 = 0.0;
    let mut slippage_ratio: f64 = 0.0;
    let mut price_key = "close".to_string();
    let mut benchmark: Option<String> = None;
    let mut out: Option<PathBuf> = None;

    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data" => data = Some(PathBuf::from(value("--data", it.next())?)),
            "--from" => from = num("--from", it.next())?,
            "--to" => to = num("--to", it.next())?,
            "--fee-ratio" => fee_ratio = num("--fee-ratio", it.next())?,
            "--slippage-ratio" => slippage_ratio = num("--slippage-ratio", it.next())?,
            "--price-key" => price_key = value("--price-key", it.next())?,
            "--benchmark" => benchmark = Some(value("--benchmark", it.next())?),
            "--out" => out = Some(PathBuf::from(value("--out", it.next())?)),
            _ if a.starts_with('-') => return Err(format!("unknown flag `{a}`")),
            _ => {
                if file.is_some() {
                    return Err("expected at most one strategy file".into());
                }
                file = Some(a);
            }
        }
    }

    let Some(data) = data else {
        return Err(
            "--data <dir> is required (a data-layout tree, see docs/data-layout.md)".into(),
        );
    };
    Ok(RunArgs {
        file,
        data,
        from,
        to,
        fee_ratio,
        slippage_ratio,
        price_key,
        benchmark,
        out,
    })
}

/// Lower one lemon source and backtest it; returns the pretty Report JSON.
/// Parse errors are path-labelled (gofmt-style); everything after the parse
/// (data loading, engine) is prefixed `lemon:`.
fn run_strategy(path: &str, src: &str, args: &RunArgs) -> Result<String, String> {
    let spec =
        lemon::parse(src).map_err(|e| format!("{path}:{}:{}: {}", e.line, e.col, e.message))?;
    let cfg = yuzu_core::backtest::BacktestConfig {
        fee_ratio: args.fee_ratio,
        slippage_ratio: args.slippage_ratio,
        benchmark_key: args.benchmark.clone(),
        ..Default::default()
    };
    let report = yuzu_research::run_single(
        &args.data,
        &spec.to_string(),
        args.from,
        args.to,
        &cfg,
        &args.price_key,
    )
    .map_err(|e| format!("lemon: {e}"))?;
    serde_json::to_string_pretty(&report).map_err(|e| format!("lemon: {e}"))
}

fn run_run(args: Vec<String>) -> ExitCode {
    let parsed = match parse_run_args(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lemon: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (path, src) = match &parsed.file {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => (p.clone(), s),
            Err(e) => {
                eprintln!("lemon: {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => match read_stdin() {
            Some(s) => ("<stdin>".to_string(), s),
            None => {
                eprintln!("lemon: failed to read stdin");
                return ExitCode::FAILURE;
            }
        },
    };
    match run_strategy(&path, &src, &parsed) {
        Ok(json) => match &parsed.out {
            Some(out) => {
                if let Err(e) = std::fs::write(out, format!("{json}\n")) {
                    eprintln!("lemon: {}: {e}", out.display());
                    return ExitCode::FAILURE;
                }
                ExitCode::SUCCESS
            }
            None => {
                println!("{json}");
                ExitCode::SUCCESS
            }
        },
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("fmt") => run_fmt(args.collect()),
        Some("lint") => run_lint(args.collect()),
        Some("check") => run_check(args.collect()),
        Some("run") => run_run(args.collect()),
        _ => {
            eprintln!(
                "usage: lemon fmt [-w] [file...]\n       lemon lint [--series a,b,c] [--series-file path] [file...]\n       lemon check [file...]\n       lemon run --data <dir> [--from N] [--to N] [--fee-ratio F] [--slippage-ratio F] [--price-key K] [--benchmark SYM] [--out path] [file]\n       (no file = read stdin)"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_source, format_source, lint_source, parse_run_args, run_strategy};
    use pomelo_data::csv_io::{write_series, OhlcvRow};

    #[test]
    fn checks_valid_and_reports_invalid_envelopes() {
        let ok = r#"{ "format": 1, "name": "M", "source": "close > sma(close, 2)" }"#;
        assert!(check_source("s.json", ok).is_ok());
        // A bad envelope surfaces path-labelled errors.
        let bad = r#"{ "format": 1, "name": "M" }"#;
        let errs = check_source("s.json", bad).unwrap_err();
        assert!(errs[0].starts_with("s.json:"), "{}", errs[0]);
    }

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

    fn args(extra: &[&str]) -> Vec<String> {
        extra.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_run_args_with_defaults_and_overrides() {
        let a = parse_run_args(args(&["s.lemon", "--data", "/tmp/d"])).unwrap();
        assert_eq!(a.file.as_deref(), Some("s.lemon"));
        assert_eq!(a.data, std::path::PathBuf::from("/tmp/d"));
        assert_eq!((a.from, a.to), (20000101, 99991231));
        assert_eq!((a.fee_ratio, a.slippage_ratio), (0.0, 0.0));
        assert_eq!(a.price_key, "close");
        assert!(a.benchmark.is_none() && a.out.is_none());

        let a = parse_run_args(args(&[
            "--data",
            "d",
            "--from",
            "20240102",
            "--to",
            "20240104",
            "--fee-ratio",
            "0.001",
            "--slippage-ratio",
            "0.0005",
            "--price-key",
            "open",
            "--benchmark",
            "SPY",
            "--out",
            "r.json",
        ]))
        .unwrap();
        assert!(a.file.is_none()); // no file = stdin
        assert_eq!((a.from, a.to), (20240102, 20240104));
        assert_eq!((a.fee_ratio, a.slippage_ratio), (0.001, 0.0005));
        assert_eq!(a.price_key, "open");
        assert_eq!(a.benchmark.as_deref(), Some("SPY"));
        assert_eq!(a.out, Some(std::path::PathBuf::from("r.json")));
    }

    #[test]
    fn rejects_bad_run_args() {
        // Every rejection names the offending flag / problem.
        let err = parse_run_args(args(&["s.lemon"])).unwrap_err();
        assert!(err.contains("--data <dir> is required"), "{err}");
        let err = parse_run_args(args(&["--data"])).unwrap_err();
        assert!(err.contains("--data needs a value"), "{err}");
        let err = parse_run_args(args(&["--data", "d", "--from", "soon"])).unwrap_err();
        assert!(err.contains("--from: invalid value `soon`"), "{err}");
        let err = parse_run_args(args(&["--data", "d", "--frmo", "1"])).unwrap_err();
        assert!(err.contains("unknown flag `--frmo`"), "{err}");
        let err = parse_run_args(args(&["a.lemon", "b.lemon", "--data", "d"])).unwrap_err();
        assert!(err.contains("at most one strategy file"), "{err}");
    }

    /// A per-test temp data tree holding prices/<sym>.csv.gz for AAA, BBB.
    fn fixture(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lemon_cli_fix_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (sym, closes) in [
            ("AAA", [10.0_f64, 11.0, 12.0]),
            ("BBB", [5.0_f64, 4.0, 6.0]),
        ] {
            let rows: Vec<OhlcvRow> = closes
                .iter()
                .enumerate()
                .map(|(i, &c)| OhlcvRow {
                    day: 20240102 + i as i32,
                    adj_open: c,
                    adj_high: c,
                    adj_low: c,
                    adj_close: c,
                    volume: 0.0,
                })
                .collect();
            let p = dir.join("prices");
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(
                p.join(format!("{sym}.csv.gz")),
                write_series(&rows).unwrap(),
            )
            .unwrap();
        }
        dir
    }

    fn run_args_for(dir: &std::path::Path) -> super::RunArgs {
        super::RunArgs {
            file: None,
            data: dir.to_path_buf(),
            from: 20240102,
            to: 20240104,
            fee_ratio: 0.0,
            slippage_ratio: 0.0,
            price_key: "close".to_string(),
            benchmark: None,
            out: None,
        }
    }

    #[test]
    fn runs_a_lemon_source_end_to_end() {
        let dir = fixture("run");
        let out = run_strategy("s.lemon", "is_largest(close, 1)", &run_args_for(&dir)).unwrap();
        // The output is the engine Report, byte-identical to lowering by hand
        // and calling the library runner directly.
        let spec = lemon::parse("is_largest(close, 1)").unwrap();
        let report = yuzu_research::run_single(
            &dir,
            &spec.to_string(),
            20240102,
            20240104,
            &Default::default(),
            "close",
        )
        .unwrap();
        assert_eq!(out, serde_json::to_string_pretty(&report).unwrap());
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["equity"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn labels_parse_errors_and_prefixes_engine_errors() {
        let dir = fixture("err");
        // Parse error: gofmt-style path:line:col label.
        let err = run_strategy("bad.lemon", "sma(close,", &run_args_for(&dir)).unwrap_err();
        assert!(err.starts_with("bad.lemon:"), "{err}");
        // Engine error (unknown series — the classic silent-Data-leaf typo).
        let err = run_strategy("typo.lemon", "clsoe > 1", &run_args_for(&dir)).unwrap_err();
        assert!(err.starts_with("lemon: "), "{err}");
        assert!(err.contains("clsoe"), "{err}");
    }
}
