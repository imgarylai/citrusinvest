// Research views for the playground: the same lemon expression, run through the
// engine's other two pipelines. `renderFactor` draws a FactorReport (quantile
// forward returns + rank-IC over time); `renderEvent` draws an EventStudy
// (average return path around the expression's 0/1 crossings). Both build plain
// DOM from the engine JSON and reuse report/charts.ts for the time series — no
// metric is computed here.

import type { EventStudy, FactorReport } from './types.ts';
import { renderChart } from './charts.ts';

// --- formatting (local mirror of render.ts's helpers) ----------------------
const isNum = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);
const pct = (v: number | null | undefined, dp = 2) => (isNum(v) ? `${(v * 100).toFixed(dp)}%` : '—');
const pctS = (v: number | null | undefined, dp = 2) =>
  isNum(v) ? `${v > 0 ? '+' : ''}${(v * 100).toFixed(dp)}%` : '—';
const num = (v: number | null | undefined, dp = 3) => (isNum(v) ? v.toFixed(dp) : '—');
const signCls = (v: number | null | undefined) => (isNum(v) ? (v > 0 ? ' pos' : v < 0 ? ' neg' : '') : '');

interface Card {
  label: string;
  value: string;
  sign?: number | null;
  sub?: string;
  tip?: string;
}
const card = (c: Card) =>
  `<div class="pg-metric"${c.tip ? ` title="${c.tip}"` : ''}>` +
  `<div class="k">${c.label}</div><div class="v${signCls(c.sign)}">${c.value}</div>` +
  (c.sub ? `<div class="s">${c.sub}</div>` : '') +
  `</div>`;
const cards = (list: (Card | null)[]) =>
  `<div class="pg-metrics pg-metrics-report">${list.filter(Boolean).map((c) => card(c!)).join('')}</div>`;
const section = (title: string, body: string, note = '') =>
  `<div class="pg-section"><h4 class="pg-section-title">${title}</h4>${body}` +
  (note ? `<p class="pg-note">${note}</p>` : '') +
  `</div>`;

// --- factor ----------------------------------------------------------------

/** Vertical bars for the per-quantile mean forward return (Q1 = lowest factor). */
function quantileBarsHtml(qr: (number | null)[]): string {
  const finite = qr.filter(isNum) as number[];
  if (finite.length === 0) return '<p class="pg-note">No quantile returns to plot.</p>';
  const maxAbs = Math.max(...finite.map((v) => Math.abs(v))) || 1;
  const cols = qr
    .map((v, i) => {
      // Center-zero diverging bar: height is a % of the half-track, so positive
      // grows up from the mid-line and negative grows down.
      const half = isNum(v) ? (Math.abs(v) / maxAbs) * 50 : 0;
      const up = isNum(v) && v >= 0;
      const bar = isNum(v) ? `<i class="${up ? 'pos' : 'neg'}" style="height:${half.toFixed(1)}%"></i>` : '';
      return (
        `<div class="pg-qbar" title="Q${i + 1}: ${pctS(v)}">` +
        `<span class="pg-qbar-val${signCls(v)}">${pctS(v, 1)}</span>` +
        `<div class="pg-qbar-slot">${bar}</div>` +
        `<span class="pg-qbar-lab">Q${i + 1}</span>` +
        `</div>`
      );
    })
    .join('');
  return `<div class="pg-qbars">${cols}</div>`;
}

export interface FactorContext {
  /** Forward-return horizon in trading days (for the labels). */
  horizon: number;
}

export function renderFactor(mount: HTMLElement, report: FactorReport, ctx: FactorContext): void {
  const periods = report.dates.length;
  const head = cards([
    { label: 'Mean IC', value: num(report.mean_ic), sign: report.mean_ic, tip: 'Average per-date Spearman rank correlation between the factor and forward returns' },
    { label: 'ICIR', value: num(report.icir, 2), sign: report.icir, tip: 'mean IC ÷ IC std — consistency of the signal (not annualized)' },
    { label: 'Long-short', value: pctS(report.long_short), sign: report.long_short, tip: 'Top-minus-bottom quantile mean forward return' },
    { label: 'Top turnover', value: pct(report.top_quantile_turnover, 0), tip: 'Share of the top bucket that leaves it each period' },
    { label: 'Periods', value: String(periods), tip: 'Dates with a defined cross-sectional IC' },
  ]);

  mount.innerHTML =
    head +
    section(
      `Mean forward return by factor quantile (${report.quantiles} buckets)`,
      quantileBarsHtml(report.quantile_returns),
      `Each bucket's average ${ctx.horizon}-day forward return. A monotonic ramp from Q1 to Q${report.quantiles} — and a positive long-short — means the factor sorts future returns.`,
    ) +
    section('Rank IC over time', `<div data-chart="ic"></div>`,
      'Per-date Spearman correlation between the factor and forward returns. A consistently positive (or negative) line is a persistent signal; noise around zero is not.');

  const el = mount.querySelector<HTMLElement>('[data-chart="ic"]');
  if (el) {
    renderChart(
      el,
      '',
      report.dates,
      [{ label: 'Rank IC', values: report.ic, colorVar: '--pg-line' }],
      { height: 200, baseline: 0, format: (v) => v.toFixed(2) },
    );
  }
}

// --- event -----------------------------------------------------------------

/** Small dependency-free chart of the cumulative path across event lags. */
function drawEventPath(canvas: HTMLCanvasElement, study: EventStudy): void {
  const cssW = canvas.clientWidth || 640;
  if (cssW < 10) return;
  const cssH = 220;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = cssW * dpr;
  canvas.height = cssH * dpr;
  canvas.style.height = `${cssH}px`;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssW, cssH);

  const styles = getComputedStyle(canvas);
  const line = styles.getPropertyValue('--pg-line').trim() || '#ffcf40';
  const grid = styles.getPropertyValue('--pg-grid').trim() || 'rgba(128,128,128,0.3)';
  const text = styles.color;
  const font = `11px ${styles.fontFamily}`;

  const cum = study.cumulative.map((v) => (isNum(v) ? v : NaN));
  const finite = cum.filter((v) => Number.isFinite(v));
  if (finite.length < 2) return;
  const pad = { l: 8, r: 8, t: 10, b: 22 };
  const w = cssW - pad.l - pad.r;
  const h = cssH - pad.t - pad.b;
  let lo = Math.min(0, ...finite);
  let hi = Math.max(0, ...finite);
  if (lo === hi) { lo -= 0.01; hi += 0.01; }
  const n = study.lags.length;
  const x = (i: number) => pad.l + (n > 1 ? (i / (n - 1)) * w : 0);
  const y = (v: number) => pad.t + h - ((v - lo) / (hi - lo)) * h;

  // zero baseline
  ctx.strokeStyle = grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(pad.l, y(0));
  ctx.lineTo(pad.l + w, y(0));
  ctx.stroke();

  // lag-0 vertical marker + per-lag x labels
  ctx.font = font;
  ctx.fillStyle = text;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'alphabetic';
  for (let i = 0; i < n; i++) {
    const lag = study.lags[i];
    if (lag === 0) {
      ctx.strokeStyle = grid;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(x(i), pad.t);
      ctx.lineTo(x(i), pad.t + h);
      ctx.stroke();
    }
    if (n <= 16 || lag % 5 === 0) {
      ctx.fillText(lag > 0 ? `+${lag}` : String(lag), x(i), cssH - 6);
    }
  }

  // cumulative path
  ctx.strokeStyle = line;
  ctx.lineWidth = 2;
  ctx.beginPath();
  let started = false;
  for (let i = 0; i < n; i++) {
    const v = cum[i];
    if (!Number.isFinite(v)) { started = false; continue; }
    if (!started) { ctx.moveTo(x(i), y(v)); started = true; } else ctx.lineTo(x(i), y(v));
  }
  ctx.stroke();

  // endpoint dot + value
  let last = n - 1;
  while (last > 0 && !Number.isFinite(cum[last])) last--;
  if (Number.isFinite(cum[last])) {
    ctx.fillStyle = line;
    ctx.beginPath();
    ctx.arc(x(last), y(cum[last]), 3, 0, Math.PI * 2);
    ctx.fill();
    ctx.textAlign = 'right';
    ctx.fillText(`${(cum[last] * 100).toFixed(2)}%`, pad.l + w, Math.max(12, y(cum[last]) - 6));
  }
}

function eventTableHtml(study: EventStudy): string {
  const rows = study.lags
    .map((lag, i) => {
      const a = study.avg_return[i];
      const c = study.cumulative[i];
      return (
        `<tr${lag === 0 ? ' class="pg-event-zero"' : ''}>` +
        `<td>${lag > 0 ? `+${lag}` : lag}</td>` +
        `<td class="${signCls(a).trim()}">${pctS(a)}</td>` +
        `<td class="${signCls(c).trim()}">${pctS(c)}</td></tr>`
      );
    })
    .join('');
  return (
    `<div class="pg-scroll"><table class="pg-table"><thead><tr>` +
    `<th>Lag (days)</th><th>Avg return</th><th>Cumulative</th>` +
    `</tr></thead><tbody>${rows}</tbody></table></div>`
  );
}

export function renderEvent(mount: HTMLElement, study: EventStudy): void {
  const zeroIdx = study.lags.indexOf(0);
  const lag0 = zeroIdx >= 0 ? study.avg_return[zeroIdx] : null;
  const end = study.cumulative[study.cumulative.length - 1] ?? null;

  const head = cards([
    { label: 'Events', value: String(study.event_count), tip: 'Number of (date, symbol) crossings with a defined return' },
    { label: 'Window', value: `−${study.pre} … +${study.post}`, tip: 'Trading days before and after each event' },
    { label: 'Day-0 avg', value: pctS(lag0), sign: lag0, tip: 'Average return on the event day itself' },
    { label: 'Cumulative', value: pctS(end), sign: end, tip: `Summed average return across the −${study.pre}…+${study.post} window` },
  ]);

  if (study.event_count === 0) {
    mount.innerHTML =
      head +
      section('Average return path', '<p class="pg-note">The expression never fired — no 0→1 crossings in the sample. Try a boolean signal, e.g. <code>close &gt; sma(close, 50)</code>.</p>');
    return;
  }

  mount.innerHTML =
    head +
    section('Average return path around the event', `<div class="pg-chartbox"><canvas class="pg-report-canvas pg-event-canvas" aria-label="Event return path"></canvas></div>`,
      'Averaged across every crossing, aligned at day 0 (vertical line). Raw returns — subtract a benchmark panel first if you want abnormal returns.') +
    section('Per-lag detail', eventTableHtml(study));

  const canvas = mount.querySelector<HTMLCanvasElement>('.pg-event-canvas');
  if (canvas) {
    drawEventPath(canvas, study);
    if (typeof ResizeObserver !== 'undefined') {
      new ResizeObserver(() => drawEventPath(canvas, study)).observe(canvas);
    }
  }
}
