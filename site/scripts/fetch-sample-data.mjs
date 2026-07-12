#!/usr/bin/env node
// Build the playground's sample dataset from REAL, freely-redistributable data.
//
//   prices  — Boris Marjanovic's "Huge Stock Market Dataset" (Kaggle),
//             licensed CC0 / public domain. Daily US OHLCV through 2017-11-10,
//             adjusted for splits and dividends.
//             https://www.kaggle.com/datasets/borismarjanovic/price-volume-data-for-all-us-stocks-etfs
//   pe      — derived: adjusted close / last reported fiscal-year diluted EPS,
//             from SEC EDGAR XBRL company facts (US government work, public
//             domain). Each 10-K's EPS becomes visible ON ITS FILING DATE, not
//             the period end — no look-ahead.
//             https://www.sec.gov/search-filings/edgar-application-programming-interfaces
//
// Almost every "free" market-data source (Yahoo, Stooq, FMP, Tiingo, Alpaca …)
// forbids redistributing the data, which rules them out for a public static
// site. This CC0 dataset is the newest daily OHLCV we found that is genuinely
// public domain — hence the window ending in 2017. The engine itself is
// data-source-agnostic; see the "Bring your own data" guide for current data.
//
// The window starts 2014-11-03: AAPL's 7:1 split (2014-06-09) means EPS filed
// before its FY2014 10-K (2014-10-27) is on a pre-split share count, while the
// Kaggle prices are fully adjusted. Starting after that filing lets us use
// as-reported EPS with no split table. Residual caveat, documented on the
// playground page: prices are dividend-adjusted, so PE early in the window is
// understated by the (few-percent) dividend adjustment factor.
//
// Network efficiency: the Kaggle archive is ~500 MB, but we only need ten
// ~100 KB members, so this script reads the zip via HTTP Range requests —
// central directory first, then just the members we want (~2 MB total).
// Set SAMPLE_STOCKS_ZIP=/path/to/archive.zip to use a local copy instead.
//
// Output shape (consumed by src/components/playground.ts) is unchanged:
//   { dates: [YYYYMMDD…], symbols, industry, panels: {close, open, high, low, volume, pe} }
//
// Run with: SEC_CONTACT=you@example.com npm run fetch:data
// (writes public/data/sample.json; behind an HTTP proxy add NODE_USE_ENV_PROXY=1)

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { inflateRawSync } from 'node:zlib';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const KAGGLE_URL =
  'https://www.kaggle.com/api/v1/datasets/download/borismarjanovic/price-volume-data-for-all-us-stocks-etfs';
// SEC requires automated clients to identify themselves with a contact email
// (https://www.sec.gov/os/accessing-edgar-data) and 403s requests without one.
const contact = process.env.SEC_CONTACT;
if (!contact || !contact.includes('@')) {
  console.error('Set SEC_CONTACT=you@example.com — SEC EDGAR requires a contact email.');
  process.exit(1);
}
const SEC_UA = `citrusquant-site sample-data script (${contact})`;

const START = '2014-11-03'; // see header: first date with post-split AAPL EPS on file

// CIKs are pinned rather than looked up from SEC's current ticker map: tickers
// drift ("XOM" now resolves to ExxonMobil Holdings Corp, a different entity
// with no 2014-17 filings). These are the issuers behind the 2014-17 prices.
const UNIVERSE = [
  { sym: 'AAPL', industry: 'Tech', cik: '0000320193' },
  { sym: 'MSFT', industry: 'Tech', cik: '0000789019' },
  { sym: 'NVDA', industry: 'Tech', cik: '0001045810' },
  { sym: 'AMZN', industry: 'Consumer', cik: '0001018724' },
  { sym: 'WMT', industry: 'Consumer', cik: '0000104169' },
  { sym: 'JPM', industry: 'Financial', cik: '0000019617' },
  { sym: 'GS', industry: 'Financial', cik: '0000886982' },
  { sym: 'XOM', industry: 'Energy', cik: '0000034088' },
  { sym: 'JNJ', industry: 'Health', cik: '0000200406' },
  { sym: 'PFE', industry: 'Health', cik: '0000078003' },
];

// Benchmark series for the report's relative metrics (alpha/beta/excess…).
// SPY lives in the same CC0 archive, under ETFs/ rather than Stocks/. When the
// bundled sample.json has no `benchmark` block (older generations), the
// playground falls back to an equal-weight basket of the universe.
const BENCHMARK = { sym: 'SPY', member: 'ETFs/spy.us.txt' };

// --- minimal zip-over-HTTP reader ------------------------------------------
// Reads specific members out of a (possibly remote) zip archive. Supports the
// classic (non-zip64) central directory, which covers this <4 GB archive, and
// only compression methods 0 (stored) and 8 (deflate).

/** Abstracts "read bytes [start, end]" over a local file or a remote URL. */
async function openByteSource() {
  const local = process.env.SAMPLE_STOCKS_ZIP;
  if (local) {
    const buf = readFileSync(local);
    console.log(`using local archive ${local} (${(buf.length / 1e6).toFixed(0)} MB)`);
    return {
      size: buf.length,
      read: async (start, end) => buf.subarray(start, end + 1),
    };
  }
  // Kaggle 302s public dataset downloads to a signed GCS URL that supports
  // Range requests; resolve it once and reuse it.
  const res = await fetch(KAGGLE_URL, { redirect: 'manual' });
  const url = res.headers.get('location');
  if (!url) throw new Error(`expected a redirect from Kaggle, got HTTP ${res.status}`);
  const read = async (start, end) => {
    const r = await fetch(url, { headers: { Range: `bytes=${start}-${end}` } });
    if (r.status !== 206) throw new Error(`range request failed: HTTP ${r.status}`);
    return Buffer.from(await r.arrayBuffer());
  };
  const probe = await fetch(url, { headers: { Range: 'bytes=0-0' } });
  if (probe.status !== 206) throw new Error(`archive URL not range-readable: HTTP ${probe.status}`);
  const size = Number(probe.headers.get('content-range').split('/')[1]);
  console.log(`remote archive: ${(size / 1e6).toFixed(0)} MB (reading only what we need)`);
  return { size, read };
}

async function readZipMembers(source, wanted) {
  // End-of-central-directory record: signature 0x06054b50, within the last
  // 22 + 65535 bytes (max comment length).
  const tail = await source.read(Math.max(0, source.size - 66000), source.size - 1);
  let eocd = -1;
  for (let i = tail.length - 22; i >= 0; i--) {
    if (tail.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error('zip: end-of-central-directory not found');
  const cdSize = tail.readUInt32LE(eocd + 12);
  const cdOffset = tail.readUInt32LE(eocd + 16);
  if (cdOffset === 0xffffffff) throw new Error('zip64 archives not supported');

  const cd = await source.read(cdOffset, cdOffset + cdSize - 1);
  const index = new Map(); // name -> {method, compSize, uncompSize, localOffset}
  for (let p = 0; p + 46 <= cd.length && cd.readUInt32LE(p) === 0x02014b50; ) {
    const method = cd.readUInt16LE(p + 10);
    let compSize = cd.readUInt32LE(p + 20);
    let uncompSize = cd.readUInt32LE(p + 24);
    const nameLen = cd.readUInt16LE(p + 28);
    const extraLen = cd.readUInt16LE(p + 30);
    const commentLen = cd.readUInt16LE(p + 32);
    let localOffset = cd.readUInt32LE(p + 42);
    const name = cd.toString('utf8', p + 46, p + 46 + nameLen);
    // 0xffffffff means "see the zip64 extra field" (id 0x0001), which lists
    // 64-bit values for exactly the fields that overflowed, in a fixed order.
    // This archive uses it even though it's <4 GB (it was written streaming).
    for (let q = p + 46 + nameLen, qEnd = q + extraLen; q + 4 <= qEnd; ) {
      const id = cd.readUInt16LE(q);
      const sz = cd.readUInt16LE(q + 2);
      if (id === 0x0001) {
        let r = q + 4;
        if (uncompSize === 0xffffffff) (uncompSize = Number(cd.readBigUInt64LE(r))), (r += 8);
        if (compSize === 0xffffffff) (compSize = Number(cd.readBigUInt64LE(r))), (r += 8);
        if (localOffset === 0xffffffff) localOffset = Number(cd.readBigUInt64LE(r));
      }
      q += 4 + sz;
    }
    index.set(name, { method, compSize, uncompSize, localOffset });
    p += 46 + nameLen + extraLen + commentLen;
  }

  const out = new Map();
  for (const name of wanted) {
    const entry = index.get(name);
    if (!entry) throw new Error(`zip: member not found: ${name}`);
    // Local file header (30 bytes fixed) to learn its name/extra lengths.
    const lh = await source.read(entry.localOffset, entry.localOffset + 29);
    if (lh.readUInt32LE(0) !== 0x04034b50) throw new Error(`zip: bad local header for ${name}`);
    const dataStart = entry.localOffset + 30 + lh.readUInt16LE(26) + lh.readUInt16LE(28);
    const raw = await source.read(dataStart, dataStart + entry.compSize - 1);
    const data = entry.method === 8 ? inflateRawSync(raw) : raw;
    if (data.length !== entry.uncompSize)
      throw new Error(`zip: ${name}: got ${data.length} bytes, expected ${entry.uncompSize}`);
    out.set(name, data);
    console.log(`  ${name}: ${(raw.length / 1024).toFixed(0)} KB compressed`);
  }
  return out;
}

// --- prices -----------------------------------------------------------------

function parseBars(text) {
  // Date,Open,High,Low,Close,Volume,OpenInt
  const bars = new Map(); // 'YYYY-MM-DD' -> {o,h,l,c,v}
  for (const line of text.split('\n').slice(1)) {
    if (!line.trim()) continue;
    const [date, o, h, l, c, v] = line.split(',');
    if (date < START) continue;
    bars.set(date, { o: +o, h: +h, l: +l, c: +c, v: +v });
  }
  return bars;
}

// --- fundamentals (SEC EDGAR) ------------------------------------------------

async function secJson(url) {
  const res = await fetch(url, { headers: { 'User-Agent': SEC_UA } });
  if (!res.ok) throw new Error(`SEC request failed: HTTP ${res.status} for ${url}`);
  return res.json();
}

/**
 * Annual diluted EPS rows as reported in 10-Ks: [{end, filed, val}], one row
 * per (fiscal-year end, filing) — later filings may restate earlier years.
 */
async function fetchAnnualEps(cik) {
  const data = await secJson(
    `https://data.sec.gov/api/xbrl/companyconcept/CIK${cik}/us-gaap/EarningsPerShareDiluted.json`,
  );
  const rows = [];
  for (const u of data.units['USD/shares']) {
    if (u.form !== '10-K' || u.fp !== 'FY' || !u.start) continue;
    // keep full-year spans only (10-Ks also restate individual quarters)
    if (Date.parse(u.end) - Date.parse(u.start) < 300 * 86400_000) continue;
    rows.push({ end: u.end, filed: u.filed, val: u.val });
  }
  rows.sort((a, b) => (a.filed < b.filed ? -1 : 1));
  return rows;
}

/**
 * EPS in effect on `date`: from all filings on file by `date`, the value for
 * the most recent fiscal year (restated value if a later filing revised it).
 */
function epsAsOf(rows, date) {
  let best = null;
  for (const r of rows) {
    if (r.filed > date) continue;
    if (!best || r.end > best.end || (r.end === best.end && r.filed > best.filed)) best = r;
  }
  return best ? best.val : null;
}

// --- main ---------------------------------------------------------------------

const symbols = UNIVERSE.map((u) => u.sym);
const industry = Object.fromEntries(UNIVERSE.map((u) => [u.sym, u.industry]));

console.log('reading prices from the CC0 Kaggle archive…');
const source = await openByteSource();
const members = await readZipMembers(source, [
  ...symbols.map((s) => `Stocks/${s.toLowerCase()}.us.txt`),
  BENCHMARK.member,
]);
const barsBySym = new Map(
  symbols.map((s) => [s, parseBars(members.get(`Stocks/${s.toLowerCase()}.us.txt`).toString())]),
);
const benchBars = parseBars(members.get(BENCHMARK.member).toString());

// Trading calendar = dates where every symbol has a bar (all ten are NYSE/
// NASDAQ megacaps, so this is just the exchange calendar).
const dates = [...barsBySym.get(symbols[0]).keys()]
  .filter((d) => symbols.every((s) => barsBySym.get(s).has(d)))
  .sort();
if (dates.length < 500) throw new Error(`suspiciously few common dates: ${dates.length}`);

console.log('fetching diluted EPS from SEC EDGAR…');
const epsBySym = new Map();
for (const u of UNIVERSE) {
  epsBySym.set(u.sym, await fetchAnnualEps(u.cik));
  console.log(`  ${u.sym}: ${epsBySym.get(u.sym).length} annual EPS rows`);
}

const round2 = (x) => Math.round(x * 100) / 100;
const panel = (pick) => dates.map((d) => symbols.map((s) => pick(barsBySym.get(s).get(d), s, d)));

const out = {
  meta: {
    synthetic: false,
    generator: 'site/scripts/fetch-sample-data.mjs',
    prices:
      'Huge Stock Market Dataset (Boris Marjanovic, Kaggle, CC0 public domain); daily bars adjusted for splits and dividends.',
    fundamentals:
      'pe = adjusted close / last reported fiscal-year diluted EPS (SEC EDGAR XBRL, public domain), visible from each 10-K filing date.',
    benchmark: `${BENCHMARK.sym} adjusted close, same CC0 dataset (${BENCHMARK.member}).`,
    days: dates.length,
    start: Number(dates[0].replaceAll('-', '')),
    end: Number(dates[dates.length - 1].replaceAll('-', '')),
    generated: new Date().toISOString().slice(0, 10),
  },
  dates: dates.map((d) => Number(d.replaceAll('-', ''))),
  symbols,
  industry,
  benchmark: {
    symbol: BENCHMARK.sym,
    close: dates.map((d) => (benchBars.has(d) ? round2(benchBars.get(d).c) : null)),
  },
  panels: {
    close: panel((b) => round2(b.c)),
    open: panel((b) => round2(b.o)),
    high: panel((b) => round2(b.h)),
    low: panel((b) => round2(b.l)),
    volume: panel((b) => b.v),
    pe: panel((b, s, d) => {
      const eps = epsAsOf(epsBySym.get(s), d);
      return eps && eps > 0 ? round2(b.c / eps) : null;
    }),
  },
};

const __dirname = dirname(fileURLToPath(import.meta.url));
const dest = resolve(__dirname, '../public/data/sample.json');
mkdirSync(dirname(dest), { recursive: true });
const json = JSON.stringify(out);
writeFileSync(dest, json);
console.log(
  `wrote ${dest} (${symbols.length} symbols x ${dates.length} days, ${dates[0]} → ${dates[dates.length - 1]}, ~${(json.length / 1024).toFixed(0)} KB)`,
);
