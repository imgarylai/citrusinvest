#!/usr/bin/env node
// Render the social-card image (public/og.png, 1200x630) from an inline SVG.
// The equity curve is real: equal-weight portfolio of the sample dataset's ten
// names, rebased to 1.0 — the same data the playground backtests.
//
// Run with: npm run gen:og   (after fetch:data; needs the site deps installed
// for sharp). The PNG is committed, so this only reruns when the design or the
// dataset changes.

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const __dirname = dirname(fileURLToPath(import.meta.url));
const sample = JSON.parse(
  readFileSync(resolve(__dirname, '../public/data/sample.json'), 'utf8'),
);

// equal-weight daily-rebalanced NAV, rebased to 1.0
const closes = sample.panels.close;
const nav = [1];
for (let i = 1; i < closes.length; i++) {
  let r = 0;
  for (let j = 0; j < sample.symbols.length; j++) r += closes[i][j] / closes[i - 1][j] - 1;
  nav.push(nav[i - 1] * (1 + r / sample.symbols.length));
}

// polyline for the lower band of the card
const W = 1200;
const H = 630;
const chart = { x: 80, y: 405, w: 1040, h: 155 };
const min = Math.min(...nav);
const max = Math.max(...nav);
const pts = nav
  .map((v, i) => {
    const x = chart.x + (i / (nav.length - 1)) * chart.w;
    const y = chart.y + chart.h - ((v - min) / (max - min)) * chart.h;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  })
  .join(' ');
const baselineY = chart.y + chart.h - ((1 - min) / (max - min)) * chart.h;

const mono = `'IBM Plex Mono', ui-monospace, 'DejaVu Sans Mono', monospace`;
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}">
  <rect width="${W}" height="${H}" fill="#15171c"/>
  <!-- citrus slice mark -->
  <g transform="translate(80,72) scale(1.6)">
    <circle cx="16" cy="16" r="15" fill="#b8860b"/>
    <circle cx="16" cy="16" r="12" fill="#ffe08a"/>
    <g stroke="#b8860b" stroke-width="2.2" stroke-linecap="round">
      <path d="M16 5.5v21M5.5 16h21M8.6 8.6l14.8 14.8M23.4 8.6L8.6 23.4"/>
    </g>
    <circle cx="16" cy="16" r="2.4" fill="#b8860b"/>
  </g>
  <text x="150" y="105" font-family="${mono}" font-size="30" fill="#e8e8e8" font-weight="600">citrusquant</text>
  <text x="80" y="180" font-family="${mono}" font-size="22" letter-spacing="6" fill="#b8860b">OPEN-SOURCE BACKTEST ENGINE · RUST · WASM</text>
  <text x="80" y="255" font-family="${mono}" font-size="46" font-weight="600" fill="#ffe08a">is_largest(sma(close, 2), 3)</text>
  <text x="80" y="310" font-family="${mono}" font-size="26" fill="#a8adb8">One expression is a whole strategy.</text>
  <text x="80" y="348" font-family="${mono}" font-size="26" fill="#a8adb8">Backtest it in your browser — no server, no signup.</text>
  <line x1="${chart.x}" y1="${baselineY.toFixed(1)}" x2="${chart.x + chart.w}" y2="${baselineY.toFixed(1)}" stroke="#3a3d45" stroke-width="2"/>
  <polyline points="${pts}" fill="none" stroke="#ffcf40" stroke-width="4" stroke-linejoin="round"/>
  <text x="${chart.x + chart.w}" y="${chart.y - 12}" font-family="${mono}" font-size="20" fill="#6b7080" text-anchor="end">real data · 2014–2017 · CC0 + SEC EDGAR</text>
  <text x="${chart.x + chart.w}" y="${H - 24}" font-family="${mono}" font-size="22" fill="#b8860b" text-anchor="end">citrusquant.com</text>
</svg>`;

const dest = resolve(__dirname, '../public/og.png');
await sharp(Buffer.from(svg)).png().toFile(dest);
console.log(`wrote ${dest}`);
