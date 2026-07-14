//! The `lemon` binary: `fmt` (gofmt-style formatter — parses first, so it
//! doubles as a syntax checker), `lint` (semantic warnings: unknown series,
//! unused `let` bindings), `check` (validate strategy envelopes), and `run`
//! (lower a strategy and backtest it over a local data-layout tree). No
//! arg-parsing dep: the surface is small enough for `std::env::args`.
//!
//! ```text
//! lemon <strategy.lemon|envelope.json> [run flags]  (short for `lemon run ...`)
//! lemon fmt   [-w] [file...]                        (no file = read stdin)
//! lemon lint  [--series a,b,c] [--series-file p] [file...]
//! lemon check [file...]                             (envelopes + .lemon front-matter)
//! lemon run   [--data <dir>] [--from N] [--to N] [--fee-ratio F]
//!             [--slippage-ratio F] [--price-key K] [--benchmark SYM]
//!             [--symbols A,B] [--sync] [--out path] [file]
//! ```
//!
//! `run` accepts a `.lemon` file (with optional `#!` front-matter, see
//! [`frontmatter`]) or a strategy-envelope JSON document; parameter precedence
//! is flag > document > `$CITRUS_DATA` env > default.
//!
//! The language crate (`lemon-lang`) stays pure and I/O-free — `yuzu-core`
//! depends on it, so the engine wiring for `run` has to live out here.

mod frontmatter;
mod sync;

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
        // With no explicit --series list, a file's own `#! require:`
        // declaration supplies the known-series set (issue #246).
        let required = if have_series { None } else { fm_require(src) };
        let known = known.or(required.as_deref());
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

/// A file's `#! require:` series list, if it has valid front-matter with one.
fn fm_require(src: &str) -> Option<Vec<String>> {
    frontmatter::parse(src).ok().and_then(|f| f.require)
}

/// Validate one document: a strategy envelope (JSON, starts with `{`) or a
/// `.lemon` source with optional `#!` front-matter. Returns rendered errors
/// (empty = ok).
fn check_source(path: &str, src: &str) -> Result<(), Vec<String>> {
    if src.trim_start().starts_with('{') {
        return lemon::envelope::check(src)
            .map(|_| ())
            .map_err(|errs| errs.into_iter().map(|e| format!("{path}: {e}")).collect());
    }
    let mut errors = Vec::new();
    if let Err(e) = frontmatter::parse(src) {
        errors.push(format!("{path}:{}: {}", e.line, e.message));
    }
    if let Err(e) = lemon::parse(src) {
        errors.push(format!("{path}:{}:{}: {}", e.line, e.col, e.message));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
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

/// `lemon run` flags. Names mirror `yuzu-cli run` so the two runners stay
/// interchangeable. Every value is optional: what the user didn't say on the
/// command line is filled from the document (`#!` front-matter or the
/// envelope), then the environment, then the defaults —
/// **flag > document > env > default**.
#[derive(Debug, Default)]
struct RunFlags {
    /// Strategy source path; `None` = stdin.
    file: Option<String>,
    /// Data-layout tree root (docs/data-layout.md); falls back to `$CITRUS_DATA`.
    data: Option<PathBuf>,
    from: Option<i32>,
    to: Option<i32>,
    fee_ratio: Option<f64>,
    slippage_ratio: Option<f64>,
    price_key: Option<String>,
    benchmark: Option<String>,
    /// Explicit symbol universe (comma-separated flag / front-matter list).
    symbols: Option<Vec<String>>,
    /// Fetch missing data via the file's `#! data-source:` before running
    /// (explicit opt-in; the key comes from the environment).
    sync: bool,
    /// Report destination; `None` = stdout.
    out: Option<PathBuf>,
}

fn parse_run_flags(args: Vec<String>) -> Result<RunFlags, String> {
    fn value(flag: &str, v: Option<String>) -> Result<String, String> {
        v.ok_or_else(|| format!("{flag} needs a value"))
    }
    fn num<T: std::str::FromStr>(flag: &str, v: Option<String>) -> Result<T, String> {
        let v = value(flag, v)?;
        v.parse()
            .map_err(|_| format!("{flag}: invalid value `{v}`"))
    }

    let mut flags = RunFlags::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data" => flags.data = Some(PathBuf::from(value("--data", it.next())?)),
            "--from" => flags.from = Some(num("--from", it.next())?),
            "--to" => flags.to = Some(num("--to", it.next())?),
            "--fee-ratio" => flags.fee_ratio = Some(num("--fee-ratio", it.next())?),
            "--slippage-ratio" => flags.slippage_ratio = Some(num("--slippage-ratio", it.next())?),
            "--price-key" => flags.price_key = Some(value("--price-key", it.next())?),
            "--benchmark" => flags.benchmark = Some(value("--benchmark", it.next())?),
            "--symbols" => {
                flags.symbols = Some(
                    frontmatter::parse_symbol_list(&value("--symbols", it.next())?)
                        .map_err(|m| format!("--symbols: {m}"))?,
                )
            }
            "--sync" => flags.sync = true,
            "--out" => flags.out = Some(PathBuf::from(value("--out", it.next())?)),
            _ if a.starts_with('-') => return Err(format!("unknown flag `{a}`")),
            _ => {
                if flags.file.is_some() {
                    return Err("expected at most one strategy file".into());
                }
                flags.file = Some(a);
            }
        }
    }
    Ok(flags)
}

/// What a runnable document contributes to the run: the lowered spec plus the
/// parameters it carries (all optional except the spec).
struct RunDoc {
    spec: serde_json::Value,
    from: Option<i32>,
    to: Option<i32>,
    symbols: Option<Vec<String>>,
    require: Option<Vec<String>>,
    data_source: Option<frontmatter::DataSource>,
    price_key: Option<String>,
    cfg: yuzu_core::backtest::BacktestConfig,
}

/// A `.lemon` source: `#!` front-matter + strategy text.
fn lemon_doc(path: &str, src: &str) -> Result<RunDoc, String> {
    let fm = frontmatter::parse(src).map_err(|e| format!("{path}:{}: {}", e.line, e.message))?;
    let spec =
        lemon::parse(src).map_err(|e| format!("{path}:{}:{}: {}", e.line, e.col, e.message))?;
    Ok(RunDoc {
        spec,
        from: fm.from,
        to: fm.to,
        symbols: fm.symbols,
        require: fm.require,
        data_source: fm.data_source,
        price_key: fm.price_key,
        cfg: fm
            .config
            .map(|c| c.to_backtest_config())
            .unwrap_or_default(),
    })
}

/// A strategy-envelope JSON document (docs/strategy-envelope.md).
fn envelope_doc(path: &str, src: &str) -> Result<RunDoc, String> {
    let checked = lemon::envelope::check(src).map_err(|errs| {
        errs.iter()
            .map(|e| format!("{path}: {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    // check() validated the shape; deserialize again for config/universe.
    let env: lemon::envelope::Envelope =
        serde_json::from_str(src).map_err(|e| format!("{path}: {e}"))?;
    let cfg = match env.config {
        Some(v) => serde_json::from_value::<frontmatter::ConfigDoc>(v)
            .map_err(|e| format!("{path}: invalid `config` object: {e}"))?
            .to_backtest_config(),
        None => Default::default(),
    };
    let (mut from, mut to, mut symbols) = (None, None, None);
    if let Some(u) = env.universe {
        if u.symbols_hint.is_some() {
            return Err(format!(
                "{path}: `universe.symbols_hint` (a named point-in-time universe) is not supported by `lemon run` yet (issue #245) — use an explicit `symbols` list, or mask on a membership panel (e.g. `mask(signal, in_sp500)`)"
            ));
        }
        let date = |field: &str, v: Option<i64>| -> Result<Option<i32>, String> {
            v.map(|d| {
                i32::try_from(d).map_err(|_| format!("{path}: universe.{field} is out of range"))
            })
            .transpose()
        };
        from = date("from", u.from)?;
        to = date("to", u.to)?;
        symbols = u.symbols;
    }
    Ok(RunDoc {
        spec: checked.spec,
        from,
        to,
        symbols,
        require: None,
        data_source: None,
        price_key: None,
        cfg,
    })
}

/// Lower one document and backtest it; returns the pretty Report JSON.
/// Front-matter and parse errors are path-labelled (gofmt-style); everything
/// after (data loading, engine) is prefixed `lemon:`. `env_data` is
/// `$CITRUS_DATA`, injected by the caller so this stays testable.
fn run_strategy(
    path: &str,
    src: &str,
    flags: &RunFlags,
    env_data: Option<String>,
) -> Result<String, String> {
    // A document starting with `{` is a strategy envelope; anything else is
    // lemon source ('{' is not valid lemon, so the sniff cannot misfire).
    let doc = if src.trim_start().starts_with('{') {
        envelope_doc(path, src)?
    } else {
        lemon_doc(path, src)?
    };

    let Some(data) = flags.data.clone().or_else(|| env_data.map(PathBuf::from)) else {
        return Err(
            "lemon: no data tree: pass --data <dir> or set $CITRUS_DATA (see docs/data-layout.md)"
                .into(),
        );
    };
    let from = flags.from.or(doc.from).unwrap_or(20000101);
    let to = flags.to.or(doc.to).unwrap_or(99991231);
    let price_key = flags
        .price_key
        .clone()
        .or(doc.price_key)
        .unwrap_or_else(|| "close".into());
    let mut cfg = doc.cfg;
    if let Some(f) = flags.fee_ratio {
        cfg.fee_ratio = f;
    }
    if let Some(s) = flags.slippage_ratio {
        cfg.slippage_ratio = s;
    }
    if let Some(b) = &flags.benchmark {
        cfg.benchmark_key = Some(b.clone());
    }
    let symbols = flags.symbols.clone().or_else(|| doc.symbols.clone());

    // Gap-check the declared universe; `--sync` (and only `--sync`) may fetch
    // what's missing via the file's declared vendor (see the sync module).
    if let Some(syms) = &symbols {
        let missing = sync::missing_symbols(&data, syms);
        if !missing.is_empty() {
            if !flags.sync {
                return Err(format!(
                    "lemon: missing price data for: {} — re-run with --sync (uses the file's `#! data-source:` and your $FMP_API_KEY), or sync manually: yuzu-cli fmp-sync --symbols {} --out {}",
                    missing.join(", "),
                    missing.join(","),
                    data.display()
                ));
            }
            let Some(source) = doc.data_source else {
                return Err(
                    "lemon: --sync needs a `#! data-source:` declaration in the strategy file (supported: fmp)"
                        .into(),
                );
            };
            let req = sync::SyncRequest {
                source,
                missing: &missing,
                from,
                to,
                include_fundamentals: sync::wants_fundamentals(&doc.spec, doc.require.as_deref()),
                root: &data,
            };
            let note = sync::sync_live(&req).map_err(|e| format!("lemon: {e}"))?;
            eprintln!("lemon: {note}");
        }
    } else if flags.sync {
        return Err(
            "lemon: --sync needs an explicit universe (`#! symbols:` or --symbols) to know what to fetch"
                .into(),
        );
    }

    let report = yuzu_research::run_single(
        &data,
        &doc.spec.to_string(),
        from,
        to,
        &cfg,
        &price_key,
        symbols.as_deref(),
    )
    .map_err(|e| format!("lemon: {e}"))?;
    serde_json::to_string_pretty(&report).map_err(|e| format!("lemon: {e}"))
}

fn run_run(args: Vec<String>) -> ExitCode {
    let flags = match parse_run_flags(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lemon: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (path, src) = match &flags.file {
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
    match run_strategy(&path, &src, &flags, std::env::var("CITRUS_DATA").ok()) {
        Ok(json) => match &flags.out {
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

/// `lemon strategy.lemon` sugar: a first argument that names a strategy file
/// (rather than a subcommand) means `run`.
fn is_runnable_file(arg: &str) -> bool {
    arg.ends_with(".lemon") || arg.ends_with(".json")
}

/// `lemon --version` output. `scripts/install.sh` runs this to verify an
/// install, so keep the `lemon <semver>` shape stable.
fn version_line() -> String {
    format!("lemon {}", env!("CARGO_PKG_VERSION"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fmt") => run_fmt(args[1..].to_vec()),
        Some("lint") => run_lint(args[1..].to_vec()),
        Some("check") => run_check(args[1..].to_vec()),
        Some("run") => run_run(args[1..].to_vec()),
        Some("--version") | Some("-V") => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Some(f) if is_runnable_file(f) => run_run(args),
        _ => {
            eprintln!(
                "usage: lemon <strategy.lemon|envelope.json> [run flags]   (short for `lemon run ...`)\n       lemon fmt [-w] [file...]\n       lemon lint [--series a,b,c] [--series-file path] [file...]\n       lemon check [file...]\n       lemon run [--data <dir>] [--from N] [--to N] [--fee-ratio F] [--slippage-ratio F] [--price-key K] [--benchmark SYM] [--symbols A,B] [--sync] [--out path] [file]\n       lemon --version\n       (no file = read stdin; --data falls back to $CITRUS_DATA)"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_source, format_source, is_runnable_file, lint_source, parse_run_flags, run_strategy,
        RunFlags,
    };
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
    fn parses_run_flags_with_absent_and_explicit_values() {
        let a = parse_run_flags(args(&["s.lemon", "--data", "/tmp/d"])).unwrap();
        assert_eq!(a.file.as_deref(), Some("s.lemon"));
        assert_eq!(a.data, Some(std::path::PathBuf::from("/tmp/d")));
        // Unset flags stay None so the document/env/default chain can fill them.
        assert!(a.from.is_none() && a.to.is_none());
        assert!(a.fee_ratio.is_none() && a.slippage_ratio.is_none());
        assert!(a.price_key.is_none() && a.benchmark.is_none() && a.out.is_none());

        let a = parse_run_flags(args(&[
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
        assert_eq!((a.from, a.to), (Some(20240102), Some(20240104)));
        assert_eq!((a.fee_ratio, a.slippage_ratio), (Some(0.001), Some(0.0005)));
        assert_eq!(a.price_key.as_deref(), Some("open"));
        assert_eq!(a.benchmark.as_deref(), Some("SPY"));
        assert_eq!(a.out, Some(std::path::PathBuf::from("r.json")));
    }

    #[test]
    fn rejects_bad_run_flags() {
        // Every rejection names the offending flag / problem.
        let err = parse_run_flags(args(&["--data"])).unwrap_err();
        assert!(err.contains("--data needs a value"), "{err}");
        let err = parse_run_flags(args(&["--data", "d", "--from", "soon"])).unwrap_err();
        assert!(err.contains("--from: invalid value `soon`"), "{err}");
        let err = parse_run_flags(args(&["--data", "d", "--frmo", "1"])).unwrap_err();
        assert!(err.contains("unknown flag `--frmo`"), "{err}");
        let err = parse_run_flags(args(&["a.lemon", "b.lemon", "--data", "d"])).unwrap_err();
        assert!(err.contains("at most one strategy file"), "{err}");
    }

    #[test]
    fn missing_data_dir_is_reported_at_run_time_with_the_env_fallback() {
        let flags = RunFlags::default();
        let err = run_strategy("s.lemon", "close > 1", &flags, None).unwrap_err();
        assert!(
            err.contains("--data") && err.contains("CITRUS_DATA"),
            "{err}"
        );
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

    fn flags_for(dir: &std::path::Path) -> RunFlags {
        RunFlags {
            data: Some(dir.to_path_buf()),
            from: Some(20240102),
            to: Some(20240104),
            ..Default::default()
        }
    }

    #[test]
    fn runs_a_lemon_source_end_to_end() {
        let dir = fixture("run");
        let out = run_strategy("s.lemon", "is_largest(close, 1)", &flags_for(&dir), None).unwrap();
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
            None,
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
        let err = run_strategy("bad.lemon", "sma(close,", &flags_for(&dir), None).unwrap_err();
        assert!(err.starts_with("bad.lemon:"), "{err}");
        // Front-matter error: path:line label.
        let err =
            run_strategy("fm.lemon", "#! nmae: X\nclose > 1", &flags_for(&dir), None).unwrap_err();
        assert!(err.starts_with("fm.lemon:1:"), "{err}");
        // Engine error (unknown series — the classic silent-Data-leaf typo).
        let err = run_strategy("typo.lemon", "clsoe > 1", &flags_for(&dir), None).unwrap_err();
        assert!(err.starts_with("lemon: "), "{err}");
        assert!(err.contains("clsoe"), "{err}");
    }

    #[test]
    fn front_matter_fills_what_flags_leave_unset_and_flags_win() {
        let dir = fixture("precedence");
        let src = "#! universe: 20240102..20240104\n#! config: { \"fee_ratio\": 0.01 }\nis_largest(close, 1)";
        // No from/to/fee flags: the document supplies them.
        let doc_flags = RunFlags {
            data: Some(dir.to_path_buf()),
            ..Default::default()
        };
        let out = run_strategy("s.lemon", src, &doc_flags, None).unwrap();
        let fee_cfg = yuzu_core::backtest::BacktestConfig {
            fee_ratio: 0.01,
            ..Default::default()
        };
        let spec = lemon::parse(src).unwrap();
        let expect = yuzu_research::run_single(
            &dir,
            &spec.to_string(),
            20240102,
            20240104,
            &fee_cfg,
            "close",
            None,
        )
        .unwrap();
        assert_eq!(out, serde_json::to_string_pretty(&expect).unwrap());

        // An explicit --fee-ratio overrides the document's config.
        let flag_flags = RunFlags {
            data: Some(dir.to_path_buf()),
            fee_ratio: Some(0.0),
            ..Default::default()
        };
        let out = run_strategy("s.lemon", src, &flag_flags, None).unwrap();
        let expect = yuzu_research::run_single(
            &dir,
            &spec.to_string(),
            20240102,
            20240104,
            &Default::default(),
            "close",
            None,
        )
        .unwrap();
        assert_eq!(out, serde_json::to_string_pretty(&expect).unwrap());
    }

    #[test]
    fn env_data_fills_in_when_the_flag_is_absent() {
        let dir = fixture("envdata");
        let flags = RunFlags {
            from: Some(20240102),
            to: Some(20240104),
            ..Default::default()
        };
        let out = run_strategy(
            "s.lemon",
            "is_largest(close, 1)",
            &flags,
            Some(dir.to_string_lossy().into_owned()),
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["equity"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn runs_an_envelope_document() {
        let dir = fixture("envelope");
        let doc = r#"{
            "format": 1,
            "name": "Top1",
            "source": "is_largest(close, 1)",
            "config": { "fee_ratio": 0.01 },
            "universe": { "from": 20240102, "to": 20240104 }
        }"#;
        let flags = RunFlags {
            data: Some(dir.to_path_buf()),
            ..Default::default()
        };
        let out = run_strategy("s.json", doc, &flags, None).unwrap();
        let fee_cfg = yuzu_core::backtest::BacktestConfig {
            fee_ratio: 0.01,
            ..Default::default()
        };
        let spec = lemon::parse("is_largest(close, 1)").unwrap();
        let expect = yuzu_research::run_single(
            &dir,
            &spec.to_string(),
            20240102,
            20240104,
            &fee_cfg,
            "close",
            None,
        )
        .unwrap();
        assert_eq!(out, serde_json::to_string_pretty(&expect).unwrap());
    }

    #[test]
    fn envelope_errors_are_actionable() {
        let dir = fixture("enverr");
        let flags = flags_for(&dir);
        // Named point-in-time universes are not implemented yet — refusing
        // beats silently running the wrong universe.
        let doc = r#"{ "format": 1, "name": "X", "source": "close > 1",
            "universe": { "symbols_hint": "sp500" } }"#;
        let err = run_strategy("s.json", doc, &flags, None).unwrap_err();
        assert!(err.contains("#245"), "{err}");
        // An explicit symbol missing from the tree is an error, not a drop.
        let doc = r#"{ "format": 1, "name": "X", "source": "close > 1",
            "universe": { "symbols": ["AAA", "ZZZ"] } }"#;
        let err = run_strategy("s.json", doc, &flags, None).unwrap_err();
        assert!(err.contains("ZZZ"), "{err}");
        // A config typo must not become a silent zero-fee run.
        let doc = r#"{ "format": 1, "name": "X", "source": "close > 1",
            "config": { "fee_ration": 0.01 } }"#;
        let err = run_strategy("s.json", doc, &flags, None).unwrap_err();
        assert!(err.contains("fee_ration"), "{err}");
        // A malformed envelope reports the check() errors, path-labelled.
        let err =
            run_strategy("s.json", r#"{ "format": 1, "name": "X" }"#, &flags, None).unwrap_err();
        assert!(err.starts_with("s.json: "), "{err}");
    }

    #[test]
    fn symbols_scope_the_universe_from_document_or_flag() {
        let dir = fixture("symbols");
        // Front-matter list: scoped to BBB, is_largest must ride BBB's
        // 5→4→6 path (the 0.8 dip proves the universe shrank to one name).
        let src = "#! symbols: BBB\nis_largest(close, 1)";
        let out = run_strategy("s.lemon", src, &flags_for(&dir), None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["equity"][1], 0.8);

        // The --symbols flag overrides the document's list.
        let mut flags = flags_for(&dir);
        flags.symbols = Some(vec!["AAA".to_string()]);
        let out = run_strategy("s.lemon", src, &flags, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["equity"][1], 1.1); // AAA: 10→11→12

        // An envelope's universe.symbols scopes the run the same way.
        let doc = r#"{ "format": 1, "name": "B", "source": "is_largest(close, 1)",
            "universe": { "from": 20240102, "to": 20240104, "symbols": ["BBB"] } }"#;
        let out = run_strategy("s.json", doc, &flags_for(&dir), None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["equity"][1], 0.8);

        // A missing symbol propagates as an actionable engine-side error.
        let err = run_strategy(
            "s.lemon",
            "#! symbols: BBB, ZZZ\nis_largest(close, 1)",
            &flags_for(&dir),
            None,
        )
        .unwrap_err();
        assert!(err.contains("ZZZ"), "{err}");
    }

    #[test]
    fn check_validates_lemon_sources_with_front_matter() {
        // A clean .lemon file (front-matter + source) passes.
        assert!(check_source("s.lemon", "#! name: X\nclose > sma(close, 2)").is_ok());
        // Front-matter and parse problems are both reported, line-labelled.
        let errs = check_source("s.lemon", "#! nmae: X\nsma(close,").unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(errs[0].starts_with("s.lemon:1:"), "{}", errs[0]);
        assert!(errs[1].starts_with("s.lemon:2:"), "{}", errs[1]);
    }

    #[test]
    fn file_sugar_matches_strategy_files_only() {
        assert!(is_runnable_file("momentum.lemon"));
        assert!(is_runnable_file("envelope.json"));
        assert!(!is_runnable_file("fmt"));
        assert!(!is_runnable_file("notes.md"));
    }

    #[test]
    fn missing_data_asks_for_sync_and_sync_needs_its_declarations() {
        let dir = fixture("syncflow");
        // BBB exists in the fixture; ZZZ does not. Without --sync the run
        // stops with an actionable pointer instead of fetching anything.
        let src = "#! symbols: BBB, ZZZ\nis_largest(close, 1)";
        let err = run_strategy("s.lemon", src, &flags_for(&dir), None).unwrap_err();
        assert!(err.contains("ZZZ") && err.contains("--sync"), "{err}");

        // --sync without a `#! data-source:` declaration is refused.
        let mut flags = flags_for(&dir);
        flags.sync = true;
        let err = run_strategy("s.lemon", src, &flags, None).unwrap_err();
        assert!(err.contains("data-source"), "{err}");

        // --sync with no explicit universe has nothing to gap-check against.
        let err = run_strategy("s.lemon", "is_largest(close, 1)", &flags, None).unwrap_err();
        assert!(err.contains("--sync") && err.contains("symbols"), "{err}");

        // A complete tree makes --sync a no-op: the run just proceeds.
        let mut ok_flags = flags_for(&dir);
        ok_flags.sync = true;
        let out = run_strategy(
            "s.lemon",
            "#! symbols: AAA, BBB\n#! data-source: fmp\nis_largest(close, 1)",
            &ok_flags,
            None,
        )
        .unwrap();
        assert!(out.contains("equity"), "{out}");
    }

    #[test]
    fn lint_uses_the_files_require_declaration() {
        // `#! require:` doubles as the lint series list (no --series needed).
        assert_eq!(
            super::fm_require("#! require: close, pe\nclose > 1").as_deref(),
            Some(&["close".to_string(), "pe".into()][..])
        );
        assert!(super::fm_require("close > 1").is_none());
        let known = super::fm_require("#! require: close\nclsoe > 1").unwrap();
        let out = lint_source("x.lemon", "#! require: close\nclsoe > 1", Some(&known)).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("unknown series `clsoe`"), "{}", out[0]);
    }

    #[test]
    fn version_line_has_the_shape_the_installer_greps() {
        let v = super::version_line();
        let rest = v.strip_prefix("lemon ").expect("starts with `lemon `");
        // A dotted version number, e.g. `0.11.0` — what install.sh reports.
        assert!(rest.split('.').count() >= 2, "{v}");
        assert!(rest.chars().next().unwrap().is_ascii_digit(), "{v}");
    }
}
