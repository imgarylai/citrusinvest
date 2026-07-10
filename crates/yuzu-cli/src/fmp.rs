//! Bring-your-own-key FMP ([Financial Modeling Prep](https://site.financialmodelingprep.com/))
//! data sync for `yuzu-cli` (issue #52).
//!
//! Direct HTTP, **no third-party FMP SDK**. Given the user's own API key, fetch
//! adjusted daily bars (and optionally annual fundamentals + a `symbol → sector`
//! industry map) and write a local tree that matches
//! [`docs/data-layout.md`](../../../docs/data-layout.md):
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (always)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (--include-fundamentals)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (--include-industry)
//! ```
//!
//! The key never leaves the machine; we neither host nor redistribute FMP data.
//! FMP lives **only** here in the CLI — never in `yuzu-core`, `yuzu-data`, or WASM
//! (the [`HttpClient`] indirection keeps the networking optional and testable).
//!
//! ## MVP scope
//!
//! Enough to backtest **price-based** strategies over a short US window: close /
//! OHLC TA and cross-section ops on a modest symbol list. Fundamentals are
//! best-effort from the annual ratios/key-metrics/growth endpoints; richer
//! fundamentals, full-universe, point-in-time, and delist honesty are out of
//! scope (see #53). Which library features an FMP Starter key can *honestly*
//! support — and which panels are missing — is documented in
//! [`docs/fmp-data-source.md`](../../../docs/fmp-data-source.md) (#51).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;
use yuzu_data::csv_io::{parse_series, write_series, Field, OhlcvRow};
use yuzu_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use yuzu_data::industry::parse_industry_csv;
use yuzu_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

/// FMP API root. The stable endpoints (`/stable/...`) are the current surface.
const FMP_BASE: &str = "https://financialmodelingprep.com";

/// Object key the industry snapshot is written under (`tracked/{name}`).
const INDUSTRY_KEY: &str = "tracked/universe.csv.gz";

// ---- HTTP indirection -------------------------------------------------------

/// A classified HTTP failure so the retry loop knows whether to back off.
#[derive(Debug, Clone)]
pub enum HttpError {
    /// A non-success HTTP status (e.g. 401 bad key, 404, 429 rate-limited, 503).
    Status(u16),
    /// A transport-level failure (DNS, TLS, connection reset, timeout, …).
    Transport(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Status(code) => write!(f, "HTTP {code}"),
            HttpError::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl HttpError {
    /// Whether retrying (after a backoff) could plausibly succeed: rate limits,
    /// server-side 5xx, and transport blips. A 4xx (bad key, bad symbol) is
    /// terminal — retrying just burns the rate budget.
    fn retryable(&self) -> bool {
        match self {
            HttpError::Transport(_) => true,
            HttpError::Status(code) => *code == 429 || (500..600).contains(code),
        }
    }
}

/// Minimal blocking HTTP GET, abstracted so the sync logic is exercised with a
/// mock in CI (no live key, no network). The real implementation is
/// [`UreqClient`], compiled only with the `fmp-sync` feature.
pub trait HttpClient {
    /// GET `url`, returning the response body on a 2xx status.
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
}

/// The real ureq-backed client. `default-features=false` on ureq means no
/// transparent gzip decode; FMP JSON is read verbatim.
#[cfg(feature = "fmp-sync")]
pub struct UreqClient;

#[cfg(feature = "fmp-sync")]
impl UreqClient {
    pub fn new() -> Self {
        UreqClient
    }
}

#[cfg(feature = "fmp-sync")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "fmp-sync")]
impl HttpClient for UreqClient {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        match ureq::get(url).call() {
            Ok(resp) => resp
                .into_body()
                .with_config()
                .limit(256 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| HttpError::Transport(e.to_string())),
            Err(ureq::Error::StatusCode(code)) => Err(HttpError::Status(code)),
            Err(e) => Err(HttpError::Transport(e.to_string())),
        }
    }
}

// ---- configuration ----------------------------------------------------------

/// How an already-present symbol tree is treated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteMode {
    /// Overwrite each symbol's files with the freshly fetched window (default).
    Overwrite,
    /// Merge fetched rows into existing files, keeping earlier history and
    /// letting fetched rows win on a date collision (extend an existing tree).
    Append,
    /// Skip any symbol that already has a `prices/{SYM}.csv.gz` — resume an
    /// interrupted multi-symbol run without refetching what landed.
    Resume,
}

/// Knobs for one [`sync`] run.
pub struct SyncConfig {
    /// Inclusive date bounds, packed `YYYYMMDD` (as everywhere else in the engine).
    pub from: i32,
    pub to: i32,
    /// Also fetch annual fundamentals → `fundamentals/{SYM}.csv.gz`.
    pub include_fundamentals: bool,
    /// Also fetch company sector → `tracked/universe.csv.gz`.
    pub include_industry: bool,
    /// Skip ETFs / mutual & closed-end funds (default on) — keep only individual
    /// stocks. Classified from the profile endpoint's `isEtf` / `isFund`.
    pub skip_non_stocks: bool,
    /// Skip symbols whose company market cap is below this, in **USD**
    /// (`0.0` = off). Read from the profile endpoint's `marketCap`. The CLI
    /// accepts unit suffixes (`1b`, `500m`) via [`parse_market_cap`].
    pub min_market_cap: f64,
    /// Max requests per minute (`0` = no throttle). FMP imposes a per-plan rate
    /// limit; set this to your plan's ceiling. Starter-class keys are commonly
    /// ~300/min — verify against your own plan.
    pub rate_limit_per_min: u32,
    /// Retries per request on a retryable error before giving up on the symbol.
    pub max_retries: u32,
    /// Base backoff **duration**; the Nth retry waits `base * 2^(N-1)` — e.g. a
    /// 2-second base gives 2s, 4s, 8s, 16s. `Duration::ZERO` disables the sleep
    /// (used by tests).
    pub backoff_base: Duration,
    /// How to treat an already-present tree.
    pub mode: WriteMode,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            from: 20000101,
            to: 99991231,
            include_fundamentals: false,
            include_industry: false,
            skip_non_stocks: true,
            min_market_cap: 0.0,
            rate_limit_per_min: 300,
            max_retries: 4,
            backoff_base: Duration::from_secs(2),
            mode: WriteMode::Overwrite,
        }
    }
}

/// What a [`sync`] run produced.
#[derive(Debug, Default)]
pub struct SyncSummary {
    /// Symbols whose price file was (re)written.
    pub symbols_written: usize,
    /// Symbols skipped because they already existed (`--resume`).
    pub symbols_skipped: usize,
    /// Symbols screened out by the ETF/fund or market-cap filters.
    pub symbols_filtered: usize,
    /// Total price rows written across all symbols.
    pub price_rows: usize,
    /// Symbols with fundamentals written.
    pub fundamentals_written: usize,
    /// Whether the industry snapshot was written.
    pub industry_written: bool,
    /// Per-symbol hard failures (symbol, redacted message). A failure on one
    /// symbol does not abort the batch.
    pub failures: Vec<(String, String)>,
}

// ---- date helpers -----------------------------------------------------------

fn i32_to_iso(d: i32) -> String {
    format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100)
}

/// Parse an FMP date (`YYYY-MM-DD`, optionally with a trailing time) to packed
/// `YYYYMMDD`.
fn iso_to_i32(s: &str) -> Option<i32> {
    s.split_whitespace()
        .next()?
        .trim()
        .replace('-', "")
        .parse()
        .ok()
}

/// Redact the `apikey` query value so it never reaches logs / error strings.
fn redact(url: &str) -> String {
    match url.find("apikey=") {
        Some(i) => {
            let start = i + "apikey=".len();
            let end = url[start..]
                .find('&')
                .map(|j| start + j)
                .unwrap_or(url.len());
            format!("{}***{}", &url[..start], &url[end..])
        }
        None => url.to_string(),
    }
}

// ---- fetcher (throttle + retry) --------------------------------------------

/// Wraps an [`HttpClient`] with the rate-limit throttle and retry/backoff loop.
struct Fetcher<'a, H: HttpClient> {
    http: &'a H,
    cfg: &'a SyncConfig,
    last_request: Cell<Option<Instant>>,
}

impl<'a, H: HttpClient> Fetcher<'a, H> {
    fn new(http: &'a H, cfg: &'a SyncConfig) -> Self {
        Fetcher {
            http,
            cfg,
            last_request: Cell::new(None),
        }
    }

    /// Sleep enough to keep under `rate_limit_per_min`.
    fn throttle(&self) {
        if self.cfg.rate_limit_per_min == 0 {
            return;
        }
        let min_interval = Duration::from_secs_f64(60.0 / self.cfg.rate_limit_per_min as f64);
        if let Some(prev) = self.last_request.get() {
            let elapsed = prev.elapsed();
            if elapsed < min_interval {
                std::thread::sleep(min_interval - elapsed);
            }
        }
        self.last_request.set(Some(Instant::now()));
    }

    /// GET with throttle + bounded exponential backoff. On success returns the
    /// body; on terminal failure returns a message with the key redacted.
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut attempt = 0u32;
        loop {
            self.throttle();
            match self.http.get(url) {
                Ok(body) => return Ok(body),
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    let wait = self.cfg.backoff_base * 2u32.pow(attempt.min(16));
                    eprintln!(
                        "  retry {}/{} after {}: {} ({})",
                        attempt + 1,
                        self.cfg.max_retries,
                        e,
                        redact(url),
                        wait.as_secs_f64()
                    );
                    std::thread::sleep(wait);
                    attempt += 1;
                }
                Err(e) => return Err(format!("{e} for {}", redact(url))),
            }
        }
    }

    /// GET and parse the body as a JSON array of row objects. FMP error payloads
    /// come back as a JSON object (`{"Error Message": ...}`) rather than an
    /// array — surface that as an error instead of silently yielding no rows.
    fn get_rows(&self, url: &str) -> Result<Vec<Value>, String> {
        let body = self.get(url)?;
        let value: Value = serde_json::from_slice(&body)
            .map_err(|e| format!("bad JSON from {}: {e}", redact(url)))?;
        match value {
            Value::Array(rows) => Ok(rows),
            Value::Object(map) => {
                // Any of the FMP error shapes: {"Error Message": "..."} etc.
                if let Some(msg) = map
                    .get("Error Message")
                    .or_else(|| map.get("error"))
                    .and_then(|v| v.as_str())
                {
                    Err(format!("FMP error: {msg}"))
                } else {
                    // A lone object is unexpected for these list endpoints.
                    Err(format!("expected a JSON array from {}", redact(url)))
                }
            }
            _ => Err(format!("expected a JSON array from {}", redact(url))),
        }
    }
}

// ---- field extraction -------------------------------------------------------

/// First present, numeric value among `keys` (case-sensitive JSON field names).
fn num(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|k| row.get(*k).and_then(Value::as_f64))
}

/// Candidate JSON field names for each [`FUNDAMENTAL_FIELDS`] entry, in the same
/// order. FMP has renamed fields across API versions, so each slot lists the
/// aliases we accept; the first present numeric one wins. Unmatched → `NaN`
/// (a "missing" factor the engine treats as absent).
const FUNDAMENTAL_KEYS: &[&[&str]] = &[
    &["priceToEarningsRatio", "peRatio", "priceEarningsRatio"], // pe
    &["priceToSalesRatio", "priceSalesRatio"],                  // ps
    &["priceToBookRatio", "priceBookValueRatio", "pbRatio"],    // pb
    &["returnOnEquity"],                                        // roe
    &["netProfitMargin", "netIncomeMargin"],                    // net_margin
    &["debtToEquityRatio", "debtEquityRatio", "debtToEquity"],  // debt_to_equity
    &["marketCap", "marketCapitalization"],                     // market_cap
    &["grossProfitMargin"],                                     // gross_margin
    &["receivablesTurnover"],                                   // receivables_turnover
    &[
        "debtToAssetsRatio",
        "debtToAssets",
        "totalDebtToTotalAssets",
    ], // debt_to_assets
    &["revenue"],                                               // revenue
    &["revenueGrowth", "growthRevenue"],                        // revenue_growth
    &["epsgrowth", "epsGrowth", "growthEPS"],                   // eps_growth
    &["operatingIncomeGrowth", "growthOperatingIncome"],        // operating_income_growth
    &["netIncomeGrowth", "growthNetIncome"],                    // net_income_growth
    &["grossProfitGrowth", "growthGrossProfit"],                // gross_profit_growth
];

// ---- endpoint URLs ----------------------------------------------------------

fn price_url(sym: &str, cfg: &SyncConfig, key: &str) -> String {
    format!(
        "{FMP_BASE}/stable/historical-price-eod/dividend-adjusted?symbol={sym}&from={}&to={}&apikey={key}",
        i32_to_iso(cfg.from),
        i32_to_iso(cfg.to)
    )
}

fn annual_url(endpoint: &str, sym: &str, key: &str) -> String {
    // A generous limit: dense forward-fill covers the trading calendar from
    // whatever annual snapshots the plan returns.
    format!("{FMP_BASE}/stable/{endpoint}?symbol={sym}&period=annual&limit=40&apikey={key}")
}

fn profile_url(sym: &str, key: &str) -> String {
    format!("{FMP_BASE}/stable/profile?symbol={sym}&apikey={key}")
}

// ---- prices -----------------------------------------------------------------

/// Parse FMP price rows into ascending [`OhlcvRow`]s, clamped to `[from, to]`.
/// Adjusted OHLC when the endpoint provides it, else the adjusted close stands
/// in for open/high/low so single-series (close) strategies still run.
fn parse_price_rows(rows: &[Value], cfg: &SyncConfig) -> Vec<OhlcvRow> {
    let mut out: Vec<OhlcvRow> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            let day = obj
                .get("date")
                .and_then(Value::as_str)
                .and_then(iso_to_i32)?;
            if day < cfg.from || day > cfg.to {
                return None;
            }
            let close = num(obj, &["adjClose", "close"])?;
            let open = num(obj, &["adjOpen", "open"]).unwrap_or(close);
            let high = num(obj, &["adjHigh", "high"]).unwrap_or(close);
            let low = num(obj, &["adjLow", "low"]).unwrap_or(close);
            let volume = num(obj, &["volume"]).unwrap_or(0.0);
            Some(OhlcvRow {
                day,
                adj_open: open,
                adj_high: high,
                adj_low: low,
                adj_close: close,
                volume,
            })
        })
        .collect();
    out.sort_by_key(|r| r.day);
    out.dedup_by_key(|r| r.day);
    out
}

/// Read the existing per-symbol price file (if any) into day-keyed rows, so
/// `--append` can merge fetched rows on top.
fn read_existing_prices(src: &LocalSource, sym: &str) -> BTreeMap<i32, OhlcvRow> {
    let key = format!("{PRICES_DIR}/{sym}.csv.gz");
    let Some(bytes) = src.get(&key).ok().flatten() else {
        return BTreeMap::new();
    };
    let col = |f| parse_series(&bytes, f).unwrap_or_default();
    let mut rows: BTreeMap<i32, OhlcvRow> = BTreeMap::new();
    for (d, v) in col(Field::AdjClose) {
        rows.entry(d).or_insert(OhlcvRow {
            day: d,
            adj_open: v,
            adj_high: v,
            adj_low: v,
            adj_close: v,
            volume: 0.0,
        });
    }
    for (d, v) in col(Field::AdjOpen) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_open = v;
        }
    }
    for (d, v) in col(Field::AdjHigh) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_high = v;
        }
    }
    for (d, v) in col(Field::AdjLow) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_low = v;
        }
    }
    for (d, v) in col(Field::Volume) {
        if let Some(r) = rows.get_mut(&d) {
            r.volume = v;
        }
    }
    rows
}

// ---- fundamentals -----------------------------------------------------------

/// Merge the annual ratios / key-metrics / financial-growth responses into one
/// object per fiscal date, then extract the [`FUNDAMENTAL_FIELDS`] factors.
/// Returns the fiscal-dated factor snapshots, ascending.
fn merge_fundamentals(bodies: &[Vec<Value>]) -> Vec<(i32, Vec<f64>)> {
    let mut by_date: BTreeMap<i32, serde_json::Map<String, Value>> = BTreeMap::new();
    for rows in bodies {
        for row in rows {
            let Some(obj) = row.as_object() else { continue };
            let Some(d) = obj.get("date").and_then(Value::as_str).and_then(iso_to_i32) else {
                continue;
            };
            let entry = by_date.entry(d).or_default();
            for (k, v) in obj {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    by_date
        .into_iter()
        .map(|(d, obj)| {
            let values = FUNDAMENTAL_KEYS
                .iter()
                .map(|keys| num(&obj, keys).unwrap_or(f64::NAN))
                .collect();
            (d, values)
        })
        .collect()
}

/// Dense forward-fill fiscal snapshots onto the trading calendar (`price_days`).
/// Each trading day carries the most recent snapshot on/before it; `report_event`
/// is `1.0` on the first trading day a new snapshot takes effect, else `0.0`.
/// Days before the first snapshot get all-`NaN` factors (engine reads as absent).
fn densify_fundamentals(snapshots: &[(i32, Vec<f64>)], price_days: &[i32]) -> Vec<FundamentalRow> {
    let nfields = FUNDAMENTAL_FIELDS.len();
    let mut rows = Vec::with_capacity(price_days.len());
    let mut si = 0usize;
    let mut current = vec![f64::NAN; nfields];
    for &day in price_days {
        let mut event = 0.0;
        while si < snapshots.len() && snapshots[si].0 <= day {
            current = snapshots[si].1.clone();
            event = 1.0;
            si += 1;
        }
        rows.push(FundamentalRow {
            day,
            values: current.clone(),
            report_event: event,
        });
    }
    rows
}

// ---- universe discovery -----------------------------------------------------

/// Fetch the full tradable-symbol universe from FMP (`/stable/stock-list`) and
/// return every ticker, sorted and de-duplicated. This is the "sync all symbols"
/// universe; the per-symbol ETF/fund and market-cap screens in [`sync`] still
/// apply. The list can be very large (thousands of names) — pair it with
/// `min_market_cap`, `rate_limit_per_min`, and `WriteMode::Resume`.
pub fn list_all_symbols<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
) -> Result<Vec<String>, String> {
    let fetcher = Fetcher::new(http, cfg);
    let rows = fetcher.get_rows(&format!("{FMP_BASE}/stable/stock-list?apikey={api_key}"))?;
    let mut syms: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.as_object()?
                .get("symbol")?
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect();
    syms.sort();
    syms.dedup();
    Ok(syms)
}

/// Filters for [`build_symbol_list`] — a screened market universe.
#[derive(Default)]
pub struct SymbolFilter {
    /// Only symbols at/above this company market cap (`0.0` = no floor).
    pub min_market_cap: f64,
    /// Restrict to one or more exchanges (comma-separated FMP codes, e.g.
    /// `NASDAQ,NYSE`). `None` = all exchanges.
    pub exchange: Option<String>,
    /// Keep ETFs / funds (default: stocks only).
    pub include_etf: bool,
    /// Cap the number of returned symbols (`None` = the API default).
    pub limit: Option<usize>,
}

/// Build a screened symbol universe from FMP's screener
/// (`/stable/company-screener`) — the "establish the sync list first" step so a
/// whole-market backtest has a persisted, reviewable symbol list to sync. The
/// filters are pushed to the API *and* re-applied client-side as a safety net.
/// Returns tickers, sorted and de-duplicated.
pub fn build_symbol_list<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
    filter: &SymbolFilter,
) -> Result<Vec<String>, String> {
    let fetcher = Fetcher::new(http, cfg);
    let mut url = format!("{FMP_BASE}/stable/company-screener?apikey={api_key}");
    if filter.min_market_cap > 0.0 {
        url.push_str(&format!(
            "&marketCapMoreThan={}",
            filter.min_market_cap as u64
        ));
    }
    if !filter.include_etf {
        url.push_str("&isEtf=false&isFund=false");
    }
    if let Some(ex) = &filter.exchange {
        url.push_str(&format!("&exchange={ex}"));
    }
    if let Some(n) = filter.limit {
        url.push_str(&format!("&limit={n}"));
    }
    let rows = fetcher.get_rows(&url)?;
    let mut syms: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            // Re-apply the screen client-side in case the API ignores a param.
            if !filter.include_etf && (flag(obj, "isEtf") || flag(obj, "isFund")) {
                return None;
            }
            if filter.min_market_cap > 0.0 {
                if let Some(mc) = num(obj, &["marketCap", "marketCapitalization"]) {
                    if mc < filter.min_market_cap {
                        return None;
                    }
                }
            }
            obj.get("symbol")?
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect();
    syms.sort();
    syms.dedup();
    Ok(syms)
}

/// Parse a market-cap threshold with an optional magnitude suffix — `k`, `m`,
/// `b`, `t` (thousand / million / billion / trillion), case-insensitive. Plain
/// numbers and scientific notation pass through. Examples: `1b` → 1e9,
/// `500m` → 5e8, `2.5t` → 2.5e12, `1e9` → 1e9, `0` → 0.
pub fn parse_market_cap(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty market-cap value".to_string());
    }
    let mult = match s.chars().last().unwrap().to_ascii_lowercase() {
        'k' => 1e3,
        'm' => 1e6,
        'b' => 1e9,
        't' => 1e12,
        _ => 1.0,
    };
    // Strip the suffix only when one matched (ASCII, so 1-byte).
    let digits = if mult == 1.0 { s } else { &s[..s.len() - 1] };
    let val: f64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("invalid market cap '{s}' (try 1b, 500m, or a plain number)"))?;
    if val < 0.0 || !val.is_finite() {
        return Err(format!("market cap must be a non-negative number: '{s}'"));
    }
    Ok(val * mult)
}

/// Parse a symbols-list file into tickers. One ticker per line; the first
/// comma-separated field is taken (so a `symbol,...` CSV works), and blank
/// lines, `#` comments, and a literal `symbol` header are skipped.
pub fn parse_symbols_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let first = line.split(',').next()?.trim();
            if first.is_empty() || first.eq_ignore_ascii_case("symbol") {
                return None;
            }
            Some(first.to_string())
        })
        .collect()
}

// ---- orchestration ----------------------------------------------------------

/// Sync `symbols` from FMP into the `out` tree per [`SyncConfig`]. Prices are
/// always fetched; fundamentals and industry are opt-in. Progress and per-symbol
/// failures are logged to stderr (with the API key redacted); a per-symbol
/// failure is recorded and the batch continues.
pub fn sync<H: HttpClient>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    out: &Path,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    if symbols.is_empty() {
        return Err("no symbols requested".to_string());
    }
    if cfg.from > cfg.to {
        return Err(format!("from ({}) is after to ({})", cfg.from, cfg.to));
    }

    let sink = LocalSource::new(out);
    let fetcher = Fetcher::new(http, cfg);
    let mut summary = SyncSummary::default();

    // For --include-industry, start from the existing snapshot so resumed /
    // skipped symbols keep their sector rows.
    let mut industry: BTreeMap<String, (String, Option<f64>)> = if cfg.include_industry {
        load_existing_industry(&sink)
    } else {
        BTreeMap::new()
    };

    // One profile GET per symbol serves the ETF/fund screen, the market-cap
    // screen, and the industry map — fetch it only when at least one needs it.
    let need_profile = cfg.skip_non_stocks || cfg.min_market_cap > 0.0 || cfg.include_industry;

    for sym in symbols {
        let price_key = format!("{PRICES_DIR}/{sym}.csv.gz");
        if cfg.mode == WriteMode::Resume && sink.get(&price_key).ok().flatten().is_some() {
            eprintln!("{sym}: already present, skipping (resume)");
            summary.symbols_skipped += 1;
            continue;
        }

        // Screen before fetching prices so filtered symbols cost no price request.
        // A profile hiccup fails *open* (keep the symbol) — a transient error on
        // the secondary endpoint must not drop the primary price sync.
        let mut profile: Option<Profile> = None;
        if need_profile {
            match fetch_profile(&fetcher, sym, api_key) {
                Ok(Some(p)) => {
                    if cfg.skip_non_stocks && (p.is_etf || p.is_fund) {
                        let kind = if p.is_etf { "ETF" } else { "fund" };
                        eprintln!("{sym}: {kind}, skipping (pass --include-etf to keep)");
                        summary.symbols_filtered += 1;
                        continue;
                    }
                    if cfg.min_market_cap > 0.0 {
                        match p.market_cap {
                            Some(mc) if mc < cfg.min_market_cap => {
                                eprintln!(
                                    "{sym}: market cap {mc:.0} < {:.0}, skipping",
                                    cfg.min_market_cap
                                );
                                summary.symbols_filtered += 1;
                                continue;
                            }
                            None => eprintln!("{sym}: market cap unknown, keeping (cannot screen)"),
                            _ => {}
                        }
                    }
                    profile = Some(p);
                }
                Ok(None) if cfg.skip_non_stocks || cfg.min_market_cap > 0.0 => {
                    eprintln!("{sym}: no profile data, cannot screen (keeping)");
                }
                Ok(None) => {}
                Err(e) => eprintln!("{sym}: profile unavailable, cannot screen (keeping): {e}"),
            }
        }

        eprintln!("{sym}: fetching prices…");
        let fetched = match fetcher
            .get_rows(&price_url(sym, cfg, api_key))
            .map(|rows| parse_price_rows(&rows, cfg))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{sym}: price fetch failed: {e}");
                summary.failures.push((sym.clone(), e));
                continue;
            }
        };
        if fetched.is_empty() {
            let msg = "no price rows in range".to_string();
            eprintln!("{sym}: {msg}");
            summary.failures.push((sym.clone(), msg));
            continue;
        }

        // Merge onto existing history when appending.
        let rows: Vec<OhlcvRow> = if cfg.mode == WriteMode::Append {
            let mut by_day = read_existing_prices(&sink, sym);
            for r in fetched {
                by_day.insert(r.day, r);
            }
            by_day.into_values().collect()
        } else {
            fetched
        };

        match write_series(&rows).map_err(|e| e.to_string()) {
            Ok(bytes) => {
                if let Err(e) = sink.put(&price_key, &bytes) {
                    let e = e.to_string();
                    eprintln!("{sym}: write failed: {e}");
                    summary.failures.push((sym.clone(), e));
                    continue;
                }
            }
            Err(e) => {
                eprintln!("{sym}: encode failed: {e}");
                summary.failures.push((sym.clone(), e));
                continue;
            }
        }
        summary.symbols_written += 1;
        summary.price_rows += rows.len();
        eprintln!("{sym}: wrote {} price rows", rows.len());

        let price_days: Vec<i32> = rows.iter().map(|r| r.day).collect();

        if cfg.include_fundamentals {
            match sync_fundamentals(&fetcher, &sink, sym, api_key, &price_days) {
                Ok(true) => summary.fundamentals_written += 1,
                Ok(false) => {}
                Err(e) => {
                    eprintln!("{sym}: fundamentals skipped: {e}");
                    summary.failures.push((format!("{sym} (fundamentals)"), e));
                }
            }
        }

        if cfg.include_industry {
            // Reuse the profile fetched above for the screen.
            match profile
                .as_ref()
                .and_then(|p| p.sector.as_ref().map(|s| (s.clone(), p.market_cap)))
            {
                Some((sector, mcap)) => {
                    industry.insert(sym.clone(), (sector, mcap));
                }
                None => eprintln!("{sym}: no sector in profile"),
            }
        }
    }

    if cfg.include_industry && !industry.is_empty() {
        let bytes = encode_industry(&industry).map_err(|e| e.to_string())?;
        sink.put(INDUSTRY_KEY, &bytes).map_err(|e| e.to_string())?;
        summary.industry_written = true;
        eprintln!("wrote {} industry rows to {INDUSTRY_KEY}", industry.len());
    }

    Ok(summary)
}

/// Fetch + write fundamentals for one symbol. Returns whether a file was
/// written (false when the plan returned no annual data at all).
fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &LocalSource,
    sym: &str,
    api_key: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{sym}: fetching fundamentals…");
    let bodies: Vec<Vec<Value>> = ["ratios", "key-metrics", "financial-growth"]
        .iter()
        .map(|ep| fetcher.get_rows(&annual_url(ep, sym, api_key)))
        .collect::<Result<_, _>>()?;
    let snapshots = merge_fundamentals(&bodies);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{sym}: wrote {} fundamental rows ({} annual snapshots)",
        rows.len(),
        snapshots.len()
    );
    Ok(true)
}

/// The bits of the company profile the sync uses: sector (for the industry
/// map), market cap (cap screen), and security-type flags (stock-only screen).
struct Profile {
    sector: Option<String>,
    market_cap: Option<f64>,
    is_etf: bool,
    is_fund: bool,
}

/// A boolean profile flag that FMP may serialize as a JSON bool or a string.
fn flag(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    match obj.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Fetch the company profile. `Ok(None)` = the endpoint returned no row for the
/// symbol (unknown ticker on this plan).
fn fetch_profile<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sym: &str,
    api_key: &str,
) -> Result<Option<Profile>, String> {
    let rows = fetcher.get_rows(&profile_url(sym, api_key))?;
    let Some(obj) = rows.first().and_then(Value::as_object) else {
        return Ok(None);
    };
    let sector = obj
        .get("sector")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Some(Profile {
        sector,
        market_cap: num(obj, &["marketCap", "marketCapitalization"]),
        is_etf: flag(obj, "isEtf"),
        is_fund: flag(obj, "isFund"),
    }))
}

/// Read the existing `tracked/universe.csv.gz` back into the accumulator so a
/// resumed run does not drop sectors for symbols it skipped.
fn load_existing_industry(src: &LocalSource) -> BTreeMap<String, (String, Option<f64>)> {
    let Some(bytes) = src.get(INDUSTRY_KEY).ok().flatten() else {
        return BTreeMap::new();
    };
    let text = decode_csv_text(&bytes);
    parse_industry_csv(&text)
        .into_iter()
        .map(|(sym, sector)| (sym, (sector, None)))
        .collect()
}

/// Decode CSV bytes that may be gzip (`.csv.gz`, what we write) or plain UTF-8.
fn decode_csv_text(bytes: &[u8]) -> String {
    use std::io::Read;
    // gzip magic 1f 8b → gunzip; otherwise treat as plain text.
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = String::new();
        if flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .is_ok()
        {
            return out;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Encode the industry map as gzip CSV: `symbol,sector,market_cap`.
fn encode_industry(
    industry: &BTreeMap<String, (String, Option<f64>)>,
) -> Result<Vec<u8>, std::io::Error> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut csv = String::from("symbol,sector,market_cap\n");
    for (sym, (sector, mcap)) in industry {
        let mcap = mcap.map(|m| m.to_string()).unwrap_or_default();
        csv.push_str(&format!("{sym},{sector},{mcap}\n"));
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(csv.as_bytes())?;
    enc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_market_cap_handles_suffixes_and_plain_numbers() {
        assert_eq!(parse_market_cap("1b").unwrap(), 1e9);
        assert_eq!(parse_market_cap("500M").unwrap(), 5e8);
        assert_eq!(parse_market_cap("2.5t").unwrap(), 2.5e12);
        assert_eq!(parse_market_cap("10k").unwrap(), 1e4);
        assert_eq!(parse_market_cap("1e9").unwrap(), 1e9);
        assert_eq!(parse_market_cap("0").unwrap(), 0.0);
        assert_eq!(parse_market_cap("1000000").unwrap(), 1e6);
        assert!(parse_market_cap("").is_err());
        assert!(parse_market_cap("1x").is_err());
        assert!(parse_market_cap("-1b").is_err());
    }

    #[test]
    fn redacts_the_api_key() {
        assert_eq!(
            redact("https://x/stable/profile?symbol=AAPL&apikey=SECRET"),
            "https://x/stable/profile?symbol=AAPL&apikey=***"
        );
        assert_eq!(
            redact("https://x/y?apikey=SECRET&from=2020-01-01"),
            "https://x/y?apikey=***&from=2020-01-01"
        );
        // No key → untouched.
        assert_eq!(redact("https://x/y?symbol=AAPL"), "https://x/y?symbol=AAPL");
    }

    #[test]
    fn iso_date_roundtrips_and_tolerates_time_suffix() {
        assert_eq!(iso_to_i32("2024-01-02"), Some(20240102));
        assert_eq!(iso_to_i32("2024-01-02 00:00:00"), Some(20240102));
        assert_eq!(i32_to_iso(20240102), "2024-01-02");
        assert_eq!(iso_to_i32("garbage"), None);
    }

    #[test]
    fn retryable_classification() {
        assert!(HttpError::Status(429).retryable());
        assert!(HttpError::Status(503).retryable());
        assert!(HttpError::Transport("reset".into()).retryable());
        assert!(!HttpError::Status(401).retryable());
        assert!(!HttpError::Status(404).retryable());
    }

    #[test]
    fn densify_forward_fills_and_marks_events() {
        // Two annual snapshots; a 4-day trading calendar straddling both.
        let snaps = vec![
            (20240102, vec![10.0; FUNDAMENTAL_FIELDS.len()]),
            (20240104, vec![20.0; FUNDAMENTAL_FIELDS.len()]),
        ];
        let days = [20240101, 20240102, 20240103, 20240104];
        let rows = densify_fundamentals(&snaps, &days);
        assert_eq!(rows.len(), 4);
        // Before the first snapshot: NaN factors, no event.
        assert!(rows[0].values[0].is_nan());
        assert_eq!(rows[0].report_event, 0.0);
        // Snapshot day: value applied, event flagged.
        assert_eq!(rows[1].values[0], 10.0);
        assert_eq!(rows[1].report_event, 1.0);
        // Between snapshots: carried forward, no event.
        assert_eq!(rows[2].values[0], 10.0);
        assert_eq!(rows[2].report_event, 0.0);
        // Second snapshot day.
        assert_eq!(rows[3].values[0], 20.0);
        assert_eq!(rows[3].report_event, 1.0);
    }

    #[test]
    fn merge_fundamentals_spreads_fields_across_endpoints() {
        let ratios = vec![serde_json::json!({"date":"2024-01-02","priceToEarningsRatio":15.0})];
        let metrics = vec![serde_json::json!({"date":"2024-01-02","marketCap":1.0e12})];
        let growth = vec![serde_json::json!({"date":"2024-01-02","revenueGrowth":0.08})];
        let snaps = merge_fundamentals(&[ratios, metrics, growth]);
        assert_eq!(snaps.len(), 1);
        let (day, vals) = &snaps[0];
        assert_eq!(*day, 20240102);
        assert_eq!(vals[0], 15.0); // pe
        assert_eq!(vals[6], 1.0e12); // market_cap
        assert_eq!(vals[11], 0.08); // revenue_growth
    }

    #[test]
    fn parse_price_rows_uses_close_fallback_and_clamps_range() {
        let cfg = SyncConfig {
            from: 20240102,
            to: 20240103,
            ..Default::default()
        };
        let rows = vec![
            // out of range (dropped)
            serde_json::json!({"date":"2024-01-01","adjClose":9.0}),
            // adjusted OHLC present
            serde_json::json!({"date":"2024-01-02","adjOpen":9.5,"adjHigh":11.0,"adjLow":9.0,"adjClose":10.0,"volume":1000}),
            // close only → OHL fall back to close, volume 0
            serde_json::json!({"date":"2024-01-03","adjClose":11.0}),
        ];
        let out = parse_price_rows(&rows, &cfg);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].day, 20240102);
        assert_eq!(out[0].adj_high, 11.0);
        assert_eq!(out[0].volume, 1000.0);
        assert_eq!(out[1].day, 20240103);
        assert_eq!(out[1].adj_open, 11.0); // fallback to close
        assert_eq!(out[1].volume, 0.0);
    }
}
