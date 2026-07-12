// Dependency-free canvas time-series chart for the report. Same approach as the
// hero equity curve: colors come from CSS custom properties so every chart
// follows the site theme, and everything is drawn with the raw 2D API.
//
// `renderChart` builds the DOM (header + canvas + hover readout) inside `box`
// and returns the draw function — callers re-invoke it when a hidden tab
// becomes visible (a display:none canvas has zero width) or on resize.

import { isoDate } from './derive.ts';

export interface ChartSeries {
  label: string;
  values: (number | null)[];
  /** CSS custom property holding the stroke color (e.g. '--pg-line'). */
  colorVar: string;
  /** Fill the area between the line and `fillBase` at ~20% alpha. */
  fill?: boolean;
}

export interface ChartOptions {
  /** Canvas CSS height, default 240. */
  height?: number;
  /** Tick/readout value formatter. */
  format?: (v: number) => string;
  /** Draw an emphasized horizontal reference at this value (e.g. 1.0 or 0). */
  baseline?: number;
  /** Force this value into the y-domain. */
  include?: number;
  /** Base value for area fills, default the baseline (or the domain min). */
  fillBase?: number;
}

const FALLBACKS: Record<string, string> = {
  '--pg-line': '#ffcf40',
  '--pg-line-2': '#6ea8fe',
  '--pg-neg': '#f26d78',
  '--pg-grid': 'rgba(128,128,128,0.3)',
};

function cssColor(el: HTMLElement, name: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || FALLBACKS[name] || '#888';
}

/** ~`count` round tick values covering [min, max]. */
function niceTicks(min: number, max: number, count = 4): number[] {
  const span = max - min || Math.abs(max) || 1;
  const step0 = span / count;
  const mag = 10 ** Math.floor(Math.log10(step0));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= step0) ?? 10 * mag;
  const out: number[] = [];
  for (let v = Math.ceil(min / step) * step; v <= max + step * 1e-6; v += step) {
    out.push(Math.abs(v) < step * 1e-6 ? 0 : v);
  }
  return out;
}

/** Redraw charts whose container was resized (one observer for all charts). */
const redraws = new WeakMap<Element, () => void>();
const resizeObserver =
  typeof ResizeObserver !== 'undefined'
    ? new ResizeObserver((entries) => {
        for (const e of entries) redraws.get(e.target)?.();
      })
    : null;

export function renderChart(
  box: HTMLElement,
  title: string,
  dates: number[],
  series: ChartSeries[],
  opts: ChartOptions = {},
): () => void {
  const height = opts.height ?? 240;
  const format = opts.format ?? ((v: number) => v.toFixed(2));

  box.classList.add('pg-chartbox');
  const legend = series
    .map(
      (s) =>
        `<span class="pg-legend-item"><i style="background:var(${s.colorVar})"></i>${s.label}</span>`,
    )
    .join('');
  box.innerHTML =
    `<div class="pg-chart-head"><span class="pg-chart-title">${title}</span>` +
    `<span class="pg-legend">${legend}</span></div>` +
    `<canvas class="pg-report-canvas" role="img" aria-label="${title}"></canvas>` +
    `<div class="pg-readout">&nbsp;</div>`;
  const canvas = box.querySelector('canvas')!;
  const readout = box.querySelector<HTMLElement>('.pg-readout')!;

  const n = dates.length;
  const pad = { l: 8, r: 8, t: 8, b: 20 };

  const finite: number[] = [];
  for (const s of series) {
    for (const v of s.values) if (v != null && Number.isFinite(v)) finite.push(v);
  }
  let lo = finite.length ? Math.min(...finite) : 0;
  let hi = finite.length ? Math.max(...finite) : 1;
  if (opts.include != null) {
    lo = Math.min(lo, opts.include);
    hi = Math.max(hi, opts.include);
  }
  if (opts.baseline != null) {
    lo = Math.min(lo, opts.baseline);
    hi = Math.max(hi, opts.baseline);
  }
  if (lo === hi) {
    lo -= 0.5;
    hi += 0.5;
  }
  const ticks = niceTicks(lo, hi);

  const showLast = () => {
    let last = n - 1;
    while (last > 0 && series.every((s) => s.values[last] == null)) last--;
    readout.innerHTML = readoutHtml(last);
  };

  const readoutHtml = (i: number) =>
    `<span class="pg-readout-date">${isoDate(dates[i])}</span>` +
    series
      .map((s) => {
        const v = s.values[i];
        return `<span class="pg-readout-val" style="color:var(${s.colorVar})">${
          v != null && Number.isFinite(v) ? format(v) : '—'
        }</span>`;
      })
      .join('');

  let hoverIdx: number | null = null;

  const draw = () => {
    const cssW = canvas.clientWidth || box.clientWidth;
    if (cssW < 10) return; // hidden tab — drawn again on activation
    const dpr = window.devicePixelRatio || 1;
    canvas.width = cssW * dpr;
    canvas.height = height * dpr;
    canvas.style.height = `${height}px`;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, height);

    const gridColor = cssColor(canvas, '--pg-grid');
    const textColor = getComputedStyle(canvas).color;
    const font = `11px ${getComputedStyle(canvas).fontFamily}`;
    const w = cssW - pad.l - pad.r;
    const h = height - pad.t - pad.b;
    const span = hi - lo;
    const x = (i: number) => pad.l + (n > 1 ? (i / (n - 1)) * w : 0);
    const y = (v: number) => pad.t + h - ((v - lo) / span) * h;

    // horizontal grid + y labels (inside the plot, above each line)
    ctx.font = font;
    ctx.textBaseline = 'bottom';
    for (const t of ticks) {
      ctx.strokeStyle = gridColor;
      ctx.lineWidth = opts.baseline != null && t === opts.baseline ? 1.5 : 0.5;
      ctx.beginPath();
      ctx.moveTo(pad.l, y(t));
      ctx.lineTo(pad.l + w, y(t));
      ctx.stroke();
      ctx.fillStyle = textColor;
      ctx.textAlign = 'left';
      ctx.fillText(format(t), pad.l + 2, y(t) - 2);
    }
    // baseline emphasis when it's not already a tick
    if (opts.baseline != null && !ticks.includes(opts.baseline)) {
      ctx.strokeStyle = gridColor;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(pad.l, y(opts.baseline));
      ctx.lineTo(pad.l + w, y(opts.baseline));
      ctx.stroke();
    }

    // x labels at each year boundary
    ctx.fillStyle = textColor;
    ctx.textBaseline = 'alphabetic';
    ctx.textAlign = 'center';
    let prevYear = Math.floor(dates[0] / 10000);
    for (let i = 1; i < n; i++) {
      const yr = Math.floor(dates[i] / 10000);
      if (yr !== prevYear) {
        ctx.strokeStyle = gridColor;
        ctx.lineWidth = 0.5;
        ctx.beginPath();
        ctx.moveTo(x(i), pad.t);
        ctx.lineTo(x(i), pad.t + h);
        ctx.stroke();
        ctx.fillText(String(yr), x(i), height - 6);
        prevYear = yr;
      }
    }

    for (const s of series) {
      const color = cssColor(canvas, s.colorVar);
      if (s.fill) {
        const base = y(opts.fillBase ?? opts.baseline ?? lo);
        ctx.fillStyle = color;
        ctx.globalAlpha = 0.16;
        ctx.beginPath();
        let open = false;
        for (let i = 0; i <= n; i++) {
          const v = i < n ? s.values[i] : null;
          if (v != null && Number.isFinite(v)) {
            if (!open) {
              ctx.moveTo(x(i), base);
              open = true;
            }
            ctx.lineTo(x(i), y(v));
          } else if (open) {
            ctx.lineTo(x(i - 1), base);
            ctx.closePath();
            open = false;
          }
        }
        ctx.fill();
        ctx.globalAlpha = 1;
      }
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.8;
      ctx.beginPath();
      let started = false;
      for (let i = 0; i < n; i++) {
        const v = s.values[i];
        if (v == null || !Number.isFinite(v)) {
          started = false;
          continue;
        }
        if (!started) {
          ctx.moveTo(x(i), y(v));
          started = true;
        } else {
          ctx.lineTo(x(i), y(v));
        }
      }
      ctx.stroke();
    }

    // crosshair
    if (hoverIdx != null) {
      ctx.strokeStyle = textColor;
      ctx.globalAlpha = 0.5;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x(hoverIdx), pad.t);
      ctx.lineTo(x(hoverIdx), pad.t + h);
      ctx.stroke();
      ctx.globalAlpha = 1;
      for (const s of series) {
        const v = s.values[hoverIdx];
        if (v == null || !Number.isFinite(v)) continue;
        ctx.fillStyle = cssColor(canvas, s.colorVar);
        ctx.beginPath();
        ctx.arc(x(hoverIdx), y(v), 3, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  };

  canvas.addEventListener('pointermove', (ev) => {
    const rect = canvas.getBoundingClientRect();
    const frac = (ev.clientX - rect.left - pad.l) / (rect.width - pad.l - pad.r);
    const i = Math.max(0, Math.min(n - 1, Math.round(frac * (n - 1))));
    if (i === hoverIdx) return;
    hoverIdx = i;
    readout.innerHTML = readoutHtml(i);
    draw();
  });
  canvas.addEventListener('pointerleave', () => {
    hoverIdx = null;
    showLast();
    draw();
  });

  showLast();
  if (resizeObserver) {
    redraws.set(box, draw);
    resizeObserver.observe(box);
  }
  draw();
  return draw;
}
