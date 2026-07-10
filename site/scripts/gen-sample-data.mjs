#!/usr/bin/env node
// Generate a small, fully SYNTHETIC market dataset for the in-browser
// playground. Nothing here is real market data, so there is no redistribution
// concern — we can commit the output straight into the public site.
//
// Output shape (consumed by src/components/playground.ts):
//   {
//     dates:   [20220103, ...],          // YYYYMMDD ints, shared by all panels
//     symbols: ["CTRA", ...],
//     industry: { "CTRA": "Tech", ... },
//     panels: { close: [[..per symbol..], ..per date..], open, high, low, volume, pe }
//   }
// The playground re-assembles each panel into the engine's
// { dates, symbols, data } request shape.
//
// Deterministic: a fixed seed means every reader of the tutorials sees the same
// numbers and the same backtest report. Re-run with `npm run gen:data`.

import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// --- deterministic PRNG (mulberry32) --------------------------------------
function mulberry32(seed) {
  let a = seed >>> 0;
  return function () {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rnd = mulberry32(0xc17205); // "citrus"
// Box–Muller standard normal
function gauss() {
  let u = 0,
    v = 0;
  while (u === 0) u = rnd();
  while (v === 0) v = rnd();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}

// --- calendar: business days across ~3 years -------------------------------
function businessDays(startISO, count) {
  const out = [];
  const d = new Date(startISO + 'T00:00:00Z');
  while (out.length < count) {
    const dow = d.getUTCDay();
    if (dow !== 0 && dow !== 6) {
      const y = d.getUTCFullYear();
      const m = d.getUTCMonth() + 1;
      const day = d.getUTCDate();
      out.push(y * 10000 + m * 100 + day);
    }
    d.setUTCDate(d.getUTCDate() + 1);
  }
  return out;
}

const N_DAYS = 756; // ~3 trading years
const dates = businessDays('2022-01-03', N_DAYS);

// --- symbols: fake tickers, each with its own drift/vol and a sector --------
const universe = [
  { sym: 'CTRA', industry: 'Tech', s0: 42, mu: 0.16, sigma: 0.30 },
  { sym: 'YUZU', industry: 'Tech', s0: 88, mu: 0.22, sigma: 0.38 },
  { sym: 'LMON', industry: 'Consumer', s0: 25, mu: 0.10, sigma: 0.24 },
  { sym: 'ORNG', industry: 'Consumer', s0: 61, mu: 0.08, sigma: 0.20 },
  { sym: 'GRPF', industry: 'Energy', s0: 34, mu: 0.05, sigma: 0.33 },
  { sym: 'PMLO', industry: 'Energy', s0: 47, mu: 0.12, sigma: 0.29 },
  { sym: 'KUMQ', industry: 'Health', s0: 120, mu: 0.14, sigma: 0.26 },
  { sym: 'TANG', industry: 'Health', s0: 18, mu: 0.18, sigma: 0.35 },
];
const symbols = universe.map((u) => u.sym);
const industry = Object.fromEntries(universe.map((u) => [u.sym, u.industry]));

const dt = 1 / 252;
// Per-symbol daily GBM close paths, plus OHLC derived around each close and a
// slowly-drifting P/E fundamental.
const closeCols = [];
const openCols = [];
const highCols = [];
const lowCols = [];
const volCols = [];
const peCols = [];

for (const u of universe) {
  const close = [];
  const open = [];
  const high = [];
  const low = [];
  const vol = [];
  const pe = [];
  let px = u.s0;
  let peLevel = 12 + rnd() * 20;
  let prevClose = u.s0;
  for (let i = 0; i < N_DAYS; i++) {
    // GBM step
    const drift = (u.mu - 0.5 * u.sigma * u.sigma) * dt;
    const shock = u.sigma * Math.sqrt(dt) * gauss();
    px = px * Math.exp(drift + shock);
    const c = round2(px);
    // open near previous close with a gap; high/low bracket the day
    const gap = prevClose * (1 + 0.004 * gauss());
    const o = round2(gap);
    const hi = round2(Math.max(o, c) * (1 + Math.abs(0.006 * gauss())));
    const lo = round2(Math.min(o, c) * (1 - Math.abs(0.006 * gauss())));
    const v = Math.round(500_000 + Math.abs(2_000_000 * (0.5 + 0.5 * gauss())));
    // P/E random-walks slowly and stays positive
    peLevel = Math.max(6, peLevel + 0.05 * gauss());
    close.push(c);
    open.push(o);
    high.push(hi);
    low.push(lo);
    vol.push(v);
    pe.push(round2(peLevel));
    prevClose = c;
  }
  closeCols.push(close);
  openCols.push(open);
  highCols.push(high);
  lowCols.push(low);
  volCols.push(vol);
  peCols.push(pe);
}

// transpose column-per-symbol -> row-per-date (data[date][symbol])
function transpose(cols) {
  const rows = [];
  for (let i = 0; i < N_DAYS; i++) {
    const row = new Array(cols.length);
    for (let j = 0; j < cols.length; j++) row[j] = cols[j][i];
    rows.push(row);
  }
  return rows;
}

function round2(x) {
  return Math.round(x * 100) / 100;
}

const out = {
  meta: {
    synthetic: true,
    generator: 'site/scripts/gen-sample-data.mjs',
    seed: 'citrus',
    note: 'Fully synthetic GBM prices — not real market data. Safe to redistribute.',
    days: N_DAYS,
    start: dates[0],
    end: dates[dates.length - 1],
  },
  dates,
  symbols,
  industry,
  panels: {
    close: transpose(closeCols),
    open: transpose(openCols),
    high: transpose(highCols),
    low: transpose(lowCols),
    volume: transpose(volCols),
    pe: transpose(peCols),
  },
};

const __dirname = dirname(fileURLToPath(import.meta.url));
const dest = resolve(__dirname, '../public/data/sample.json');
mkdirSync(dirname(dest), { recursive: true });
writeFileSync(dest, JSON.stringify(out));
const kb = (JSON.stringify(out).length / 1024).toFixed(0);
console.log(`wrote ${dest} (${symbols.length} symbols x ${N_DAYS} days, ~${kb} KB)`);
