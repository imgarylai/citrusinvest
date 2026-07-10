// Client logic for the in-browser backtest playground.
//
// Flow: sample panels JSON  ->  lemon-wasm.parse(src) -> spec (Expr tree)
//       -> yuzu-wasm.run_backtest({spec, panels, industry, price_key, config})
//       -> Report {dates, equity, metrics}  -> canvas equity curve + metric tiles
//
// The two WASM packages are NOT bundled by Vite. CI drops the wasm-pack `web`
// output into `public/wasm/{lemon,yuzu}/`, and we import it at runtime from that
// URL so the site build never needs a Rust/wasm toolchain. When the WASM is
// absent (e.g. a plain `astro build` with no CI step), the UI says so instead of
// throwing.

import { createLemonEditor } from './lemon-editor.ts';

interface SampleData {
  dates: number[];
  symbols: string[];
  industry: Record<string, string>;
  panels: Record<string, (number | null)[][]>;
}

interface Report {
  dates: number[];
  equity: number[];
  metrics: Record<string, number | null>;
}

const BASE: string = (import.meta as { env: { BASE_URL: string } }).env.BASE_URL;

let sample: SampleData | null = null;
// lemon/yuzu wasm modules. lemon is loaded eagerly (it drives editor
// highlighting via tokens()); yuzu is loaded on the first Run.
interface LemonMod {
  parse(src: string): string;
  tokens(src: string): string;
}
let lemonMod: LemonMod | null = null;
let yuzuMod: { run_backtest(input: string): string } | null = null;

async function loadLemon(): Promise<LemonMod> {
  if (lemonMod) return lemonMod;
  const url = new URL(`${BASE}wasm/lemon/lemon.js`, location.href).href;
  const lemon = await import(/* @vite-ignore */ url);
  await lemon.default();
  lemonMod = lemon;
  return lemon;
}

async function loadYuzu(): Promise<void> {
  if (yuzuMod) return;
  const url = new URL(`${BASE}wasm/yuzu/yuzu.js`, location.href).href;
  const yuzu = await import(/* @vite-ignore */ url);
  await yuzu.default();
  yuzuMod = yuzu;
}

async function loadWasm(): Promise<void> {
  await Promise.all([loadLemon(), loadYuzu()]);
}

async function loadSample(): Promise<SampleData> {
  if (sample) return sample;
  const res = await fetch(`${BASE}data/sample.json`);
  if (!res.ok) throw new Error(`could not load sample data (${res.status})`);
  sample = (await res.json()) as SampleData;
  return sample;
}

function panelRequest(s: SampleData, name: string) {
  return { dates: s.dates, symbols: s.symbols, data: s.panels[name] };
}

const PCT = new Set([
  'total_return',
  'cagr',
  'ann_volatility',
  'max_drawdown',
  'win_rate',
  'time_in_market',
  'avg_exposure',
  'best_day',
  'worst_day',
]);

const METRIC_TILES: [string, string][] = [
  ['total_return', 'Total return'],
  ['cagr', 'CAGR'],
  ['sharpe', 'Sharpe'],
  ['sortino', 'Sortino'],
  ['max_drawdown', 'Max drawdown'],
  ['ann_volatility', 'Ann. vol'],
  ['calmar', 'Calmar'],
  ['win_rate', 'Win rate'],
  ['num_trades', 'Trades'],
  ['time_in_market', 'Time in mkt'],
];

function fmt(key: string, v: number | null | undefined): string {
  if (v == null || Number.isNaN(v)) return '—';
  if (PCT.has(key)) return `${(v * 100).toFixed(1)}%`;
  if (key === 'num_trades') return String(Math.round(v));
  return v.toFixed(2);
}

function drawEquity(canvas: HTMLCanvasElement, report: Report): void {
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.clientWidth || 640;
  const cssH = 280;
  canvas.width = cssW * dpr;
  canvas.height = cssH * dpr;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, cssW, cssH);

  const eq = report.equity.filter((x) => Number.isFinite(x));
  if (eq.length < 2) return;
  const pad = { l: 8, r: 8, t: 12, b: 20 };
  const w = cssW - pad.l - pad.r;
  const h = cssH - pad.t - pad.b;
  const min = Math.min(...eq);
  const max = Math.max(...eq);
  const span = max - min || 1;
  const n = report.equity.length;
  const x = (i: number) => pad.l + (i / (n - 1)) * w;
  const y = (v: number) => pad.t + h - ((v - min) / span) * h;

  // baseline at 1.0 (rebased start)
  ctx.strokeStyle = 'rgba(255,255,255,0.15)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(pad.l, y(1));
  ctx.lineTo(pad.l + w, y(1));
  ctx.stroke();

  // equity line
  ctx.strokeStyle = '#ffcf40';
  ctx.lineWidth = 2;
  ctx.beginPath();
  let started = false;
  for (let i = 0; i < n; i++) {
    const v = report.equity[i];
    if (!Number.isFinite(v)) continue;
    const px = x(i);
    const py = y(v);
    if (!started) {
      ctx.moveTo(px, py);
      started = true;
    } else {
      ctx.lineTo(px, py);
    }
  }
  ctx.stroke();

  // end label
  const last = report.equity[n - 1];
  ctx.fillStyle = '#ffcf40';
  ctx.font = '12px system-ui, sans-serif';
  ctx.textAlign = 'right';
  ctx.fillText(`${last.toFixed(2)}×`, pad.l + w, y(last) - 6);
}

function setStatus(el: HTMLElement, msg: string, kind: 'info' | 'error'): void {
  el.textContent = msg;
  el.dataset.kind = kind;
}

export function initPlayground(root: HTMLElement): void {
  const editorEl = root.querySelector<HTMLElement>('.pg-editor')!;
  const runBtn = root.querySelector<HTMLButtonElement>('.pg-run')!;
  const status = root.querySelector<HTMLElement>('.pg-status')!;
  const metricsEl = root.querySelector<HTMLElement>('.pg-metrics')!;
  const canvas = root.querySelector<HTMLCanvasElement>('.pg-chart')!;

  const editor = createLemonEditor(
    editorEl,
    editorEl.dataset.initial ?? 'is_largest(sma(close, 2), 3)',
    () => run(),
  );

  // Load the lemon WASM up front so the editor highlights from the engine's own
  // lexer (tokens()) immediately — independent of running a backtest. Failure
  // here is non-fatal: the editor just stays uncolored.
  loadLemon()
    .then((m) => editor.setTokenizer((src) => JSON.parse(m.tokens(src))))
    .catch(() => {});

  async function run() {
    runBtn.disabled = true;
    metricsEl.innerHTML = '';
    try {
      setStatus(status, 'Loading engine + data…', 'info');
      const [s] = await Promise.all([loadSample(), loadWasm()]);

      const parsed = JSON.parse(lemonMod!.parse(editor.getValue()));
      if (!parsed.ok) {
        const e = parsed.error;
        setStatus(status, `Syntax error (line ${e.line}, col ${e.col}): ${e.message}`, 'error');
        return;
      }

      const input = {
        spec: parsed.value,
        price_key: 'close',
        industry: s.industry,
        panels: {
          close: panelRequest(s, 'close'),
          open: panelRequest(s, 'open'),
          high: panelRequest(s, 'high'),
          low: panelRequest(s, 'low'),
          volume: panelRequest(s, 'volume'),
          pe: panelRequest(s, 'pe'),
        },
        config: { fee_ratio: 0.001 },
      };

      setStatus(status, 'Running backtest…', 'info');
      const report = JSON.parse(yuzuMod!.run_backtest(JSON.stringify(input))) as Report;

      drawEquity(canvas, report);
      metricsEl.innerHTML = METRIC_TILES.map(
        ([k, label]) =>
          `<div class="pg-metric"><div class="k">${label}</div><div class="v">${fmt(
            k,
            report.metrics[k] as number,
          )}</div></div>`,
      ).join('');
      const first = report.dates[0];
      const lastD = report.dates[report.dates.length - 1];
      setStatus(status, `Done — ${report.dates.length} trading days (${first} → ${lastD}).`, 'info');
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/wasm|import|fetch|\.js/i.test(msg)) {
        setStatus(
          status,
          'Engine WASM not found. Build it with scripts/build-*-wasm.sh into site/public/wasm/ (CI does this automatically).',
          'error',
        );
      } else {
        setStatus(status, msg, 'error');
      }
    } finally {
      runBtn.disabled = false;
    }
  }

  runBtn.addEventListener('click', run);
}
