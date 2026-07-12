// Client logic for the in-browser backtest playground.
//
// Flow: sample panels JSON  ->  lemon-wasm.parse(src) -> spec (Expr tree)
//       -> yuzu-wasm.run_backtest({spec, panels, industry, price_key, config})
//       -> Report  -> canvas equity curve + metric tiles
//                  -> full tabbed report (report/render.ts) when the widget
//                     has a `.pg-report` mount (`<Playground full />`)
//
// The two WASM packages are the published npm artifacts (@citrusquant/*-wasm,
// wasm-pack `bundler` target), so Vite bundles them at build time — the same
// single-source artifact citrus-fund consumes. No Rust/wasm toolchain is needed
// to build the site (see vite-plugin-wasm in astro.config.mjs).

import { createLemonEditor } from './lemon-editor.ts';
import type { Report } from './report/types.ts';
import { equalWeightCurve } from './report/derive.ts';
import { renderReport } from './report/render.ts';

interface SampleData {
  dates: number[];
  symbols: string[];
  industry: Record<string, string>;
  panels: Record<string, (number | null)[][]>;
  /** Optional real index series (SPY) — regenerate sample.json to add it; the
   *  playground falls back to an equal-weight basket of the universe. */
  benchmark?: { symbol: string; close: (number | null)[] };
}

const BASE: string = (import.meta as { env: { BASE_URL: string } }).env.BASE_URL;

let sample: SampleData | null = null;
// lemon/yuzu wasm modules. Both are dynamically imported so Vite splits each
// into its own chunk: lemon loads eagerly on mount (it drives editor
// highlighting via tokens()); yuzu loads on the first Run. The bundler-target
// packages instantiate their own wasm on import — no manual init() step.
interface LemonMod {
  parse(src: string): string;
  tokens(src: string): string;
}
let lemonMod: LemonMod | null = null;
let yuzuMod: { run_backtest(input: string): string } | null = null;

async function loadLemon(): Promise<LemonMod> {
  if (lemonMod) return lemonMod;
  lemonMod = await import('@citrusquant/lemon-wasm');
  return lemonMod;
}

async function loadYuzu(): Promise<void> {
  if (yuzuMod) return;
  yuzuMod = await import('@citrusquant/yuzu-wasm');
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

const reducedMotion = () =>
  typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

/**
 * Draw the equity curve. Colors come from CSS custom properties on the canvas
 * (--pg-line, --pg-grid, --pg-canvas-bg) so the chart follows the site theme.
 * On first draw the line sweeps in left-to-right (skipped for reduced motion).
 */
function drawEquity(canvas: HTMLCanvasElement, report: Report, animate: boolean): void {
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.clientWidth || 640;
  const cssH = 280;
  canvas.width = cssW * dpr;
  canvas.height = cssH * dpr;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const styles = getComputedStyle(canvas);
  const lineColor = styles.getPropertyValue('--pg-line').trim() || '#b8860b';
  const gridColor = styles.getPropertyValue('--pg-grid').trim() || 'rgba(128,128,128,0.3)';

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

  const drawUpTo = (frac: number) => {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    // baseline at 1.0 (rebased start)
    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(pad.l, y(1));
    ctx.lineTo(pad.l + w, y(1));
    ctx.stroke();

    const last = Math.max(2, Math.ceil(n * frac));
    ctx.strokeStyle = lineColor;
    ctx.lineWidth = 2;
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < last; i++) {
      const v = report.equity[i];
      if (!Number.isFinite(v)) continue;
      if (!started) {
        ctx.moveTo(x(i), y(v));
        started = true;
      } else {
        ctx.lineTo(x(i), y(v));
      }
    }
    ctx.stroke();

    if (frac >= 1) {
      const end = report.equity[n - 1];
      ctx.fillStyle = lineColor;
      ctx.font = '12px ui-monospace, monospace';
      ctx.textAlign = 'right';
      ctx.fillText(`${end.toFixed(2)}×`, pad.l + w, Math.max(12, y(end) - 6));
    }
  };

  if (!animate || reducedMotion()) {
    drawUpTo(1);
    return;
  }
  const t0 = performance.now();
  const DURATION = 550;
  const step = (t: number) => {
    const frac = Math.min(1, (t - t0) / DURATION);
    drawUpTo(frac);
    if (frac < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
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
  // Present only on <Playground full /> — mounts the tabbed report.
  const reportEl = root.querySelector<HTMLElement>('.pg-report');
  let firstDraw = true;

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

      // Benchmark: the real index series when the sample data ships one,
      // otherwise a daily-rebalanced equal-weight basket of the same universe
      // (the natural "do nothing clever" alternative for a fixed universe).
      const benchLabel = s.benchmark?.symbol ?? 'EW universe';
      const benchClose = s.benchmark?.close ?? equalWeightCurve(s.panels.close);

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
          benchmark: { dates: s.dates, symbols: [benchLabel], data: benchClose.map((v) => [v]) },
        },
        config: { fee_ratio: 0.001, benchmark_key: 'benchmark', bootstrap_samples: 200 },
      };

      setStatus(status, 'Running backtest…', 'info');
      const t0 = performance.now();
      const report = JSON.parse(yuzuMod!.run_backtest(JSON.stringify(input))) as Report;
      const ms = performance.now() - t0;

      drawEquity(canvas, report, firstDraw);
      firstDraw = false;
      if (reportEl) {
        // Full mode: the tabbed report below carries all the numbers — the
        // compact tile strip would just repeat its headline.
        reportEl.hidden = false;
        renderReport(reportEl, report, { benchmarkLabel: benchLabel });
      } else {
        const metricValues = report.metrics as unknown as Record<string, number | null>;
        metricsEl.innerHTML = METRIC_TILES.map(
          ([k, label]) =>
            `<div class="pg-metric"><div class="k">${label}</div><div class="v">${fmt(
              k,
              metricValues[k],
            )}</div></div>`,
        ).join('');
      }
      const first = report.dates[0];
      const lastD = report.dates[report.dates.length - 1];
      setStatus(
        status,
        `Done in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms — ${report.dates.length} trading days (${first} → ${lastD}), in your browser.`,
        'info',
      );
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/wasm|import|fetch|\.js/i.test(msg)) {
        setStatus(
          status,
          'Could not load the backtest engine (WASM). Try reloading the page.',
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

  // Example chips: any element with data-pg-example anywhere on the page loads
  // its strategy into the editor and runs it.
  document.addEventListener('click', (ev) => {
    const chip = (ev.target as HTMLElement).closest<HTMLElement>('[data-pg-example]');
    if (!chip) return;
    ev.preventDefault();
    editor.setValue(chip.dataset.pgExample ?? '');
    root.scrollIntoView({ behavior: reducedMotion() ? 'auto' : 'smooth', block: 'center' });
    run();
  });

  // Landing-page embeds auto-run once so the hero is alive without a click.
  if (root.dataset.autorun !== undefined) {
    const kick = () => run();
    'requestIdleCallback' in window ? requestIdleCallback(kick, { timeout: 1500 }) : setTimeout(kick, 200);
  }
}
