// Pure presentation-series helpers derived from the engine Report. The engine
// computes every headline number (metrics.rs); this module only reshapes series
// for display — nothing here re-derives a metric the engine already reports.

import type { PeriodReturn, Trade } from './types.ts';

/** '20141103' → '2014-11-03'. */
export function isoDate(yyyymmdd: number): string {
  const s = String(yyyymmdd);
  return `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`;
}

/** One underwater episode: a maximal run of days with drawdown < 0. */
export interface DrawdownEpisode {
  /** Date of the peak (last day at the high before going underwater). */
  start: number;
  /** Date of the deepest point. */
  trough: number;
  /** Date the curve recovered to the prior peak, or null if still underwater at the end. */
  end: number | null;
  /** Most negative drawdown in the episode (≤ 0). */
  depth: number;
  /** Trading days spent underwater (peak → recovery, exclusive of the peak day). */
  lengthDays: number;
  /** Trading days from trough to recovery, or null while unrecovered. */
  recoveryDays: number | null;
}

/** Underwater episodes, deepest first. */
export function drawdownEpisodes(dates: number[], drawdown: number[], top = 5): DrawdownEpisode[] {
  const out: DrawdownEpisode[] = [];
  let i = 0;
  while (i < drawdown.length) {
    if (!(drawdown[i] < 0)) {
      i++;
      continue;
    }
    const startIdx = i; // first underwater day; the peak is the day before
    let troughIdx = i;
    while (i < drawdown.length && drawdown[i] < 0) {
      if (drawdown[i] < drawdown[troughIdx]) troughIdx = i;
      i++;
    }
    const recovered = i < drawdown.length; // hit a 0 row again
    out.push({
      start: dates[Math.max(0, startIdx - 1)],
      trough: dates[troughIdx],
      end: recovered ? dates[i] : null,
      depth: drawdown[troughIdx],
      lengthDays: (recovered ? i : drawdown.length) - startIdx,
      recoveryDays: recovered ? i - troughIdx : null,
    });
  }
  out.sort((a, b) => a.depth - b.depth);
  return out.slice(0, top);
}

export interface HistogramBin {
  x0: number;
  x1: number;
  count: number;
}

/**
 * Per-trade return distribution in equal-width bins aligned on 0. Bin width
 * starts at 2% and widens (2/5/10/20/50%…) until the range fits in ~30 bins,
 * so a single outsized winner can't squeeze the rest into invisibility.
 */
export function tradeHistogram(trades: Trade[], maxBins = 30): HistogramBin[] {
  const rets = trades.map((t) => t.ret).filter((r) => Number.isFinite(r));
  if (rets.length === 0) return [];
  const min = Math.min(...rets);
  const max = Math.max(...rets);
  let binWidth = 0.02;
  const widths = [0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5];
  for (const w of widths) {
    binWidth = w;
    if ((max - min) / w <= maxBins) break;
  }
  const lo = Math.floor(min / binWidth);
  const hi = Math.floor(max / binWidth);
  const bins: HistogramBin[] = [];
  for (let b = lo; b <= hi; b++) bins.push({ x0: b * binWidth, x1: (b + 1) * binWidth, count: 0 });
  for (const r of rets) bins[Math.floor(r / binWidth) - lo].count++;
  return bins;
}

/**
 * Calendar-year returns of a (possibly null-prefixed) rebased curve — used to
 * put the benchmark on the same yearly table the engine emits for the strategy.
 * Chains each year's last valid value off the previous year's.
 */
export function yearlyFromCurve(dates: number[], curve: (number | null)[]): PeriodReturn[] {
  const lastBy = new Map<number, number>(); // year → last valid value
  for (let i = 0; i < dates.length; i++) {
    const v = curve[i];
    if (v == null || !Number.isFinite(v)) continue;
    lastBy.set(Math.floor(dates[i] / 10000), v);
  }
  const years = [...lastBy.keys()].sort();
  const out: PeriodReturn[] = [];
  let prev: number | null = null;
  for (const y of years) {
    const v = lastBy.get(y)!;
    // First year chains off the curve's first valid value, matching the
    // engine's convention for a partial first bucket.
    if (prev == null) {
      const first = curve.find((x): x is number => x != null && Number.isFinite(x)) ?? v;
      out.push({ period: String(y), ret: first !== 0 ? v / first - 1 : null });
    } else {
      out.push({ period: String(y), ret: prev !== 0 ? v / prev - 1 : null });
    }
    prev = v;
  }
  return out;
}

export interface AnnualRow {
  year: string;
  strategy: number | null;
  benchmark: number | null;
  excess: number | null;
}

/** Join strategy and benchmark yearly returns on the year. */
export function annualRows(strategy: PeriodReturn[], benchmark: PeriodReturn[]): AnnualRow[] {
  const bench = new Map(benchmark.map((p) => [p.period, p.ret]));
  return strategy.map((p) => {
    const b = bench.get(p.period) ?? null;
    return {
      year: p.period,
      strategy: p.ret,
      benchmark: b,
      excess: p.ret != null && b != null ? p.ret - b : null,
    };
  });
}

/** Fraction of days the (rebased) strategy sits at or above the benchmark curve. */
export function pctDaysBeating(equity: number[], benchmark: (number | null)[]): number | null {
  let n = 0;
  let won = 0;
  for (let i = 0; i < equity.length; i++) {
    const b = benchmark[i];
    if (b == null || !Number.isFinite(b) || !Number.isFinite(equity[i])) continue;
    n++;
    if (equity[i] >= b) won++;
  }
  return n > 0 ? won / n : null;
}

/**
 * Rolling Pearson correlation of daily returns between strategy and benchmark.
 * Same 252-day window as the engine's rolling series; null until it fills.
 */
export function rollingCorrelation(
  equity: number[],
  benchmark: (number | null)[],
  window = 252,
): (number | null)[] {
  const n = equity.length;
  const out: (number | null)[] = new Array(n).fill(null);
  const re: (number | null)[] = new Array(n).fill(null);
  const rb: (number | null)[] = new Array(n).fill(null);
  for (let i = 1; i < n; i++) {
    const b0 = benchmark[i - 1];
    const b1 = benchmark[i];
    if (Number.isFinite(equity[i]) && Number.isFinite(equity[i - 1]) && equity[i - 1] !== 0) {
      re[i] = equity[i] / equity[i - 1] - 1;
    }
    if (b0 != null && b1 != null && Number.isFinite(b0) && Number.isFinite(b1) && b0 !== 0) {
      rb[i] = b1 / b0 - 1;
    }
  }
  for (let i = window; i < n; i++) {
    let se = 0;
    let sb = 0;
    let k = 0;
    for (let j = i - window + 1; j <= i; j++) {
      if (re[j] == null || rb[j] == null) continue;
      se += re[j]!;
      sb += rb[j]!;
      k++;
    }
    if (k < window * 0.9) continue; // window mostly valid or nothing
    const me = se / k;
    const mb = sb / k;
    let cov = 0;
    let ve = 0;
    let vb = 0;
    for (let j = i - window + 1; j <= i; j++) {
      if (re[j] == null || rb[j] == null) continue;
      cov += (re[j]! - me) * (rb[j]! - mb);
      ve += (re[j]! - me) ** 2;
      vb += (rb[j]! - mb) ** 2;
    }
    const denom = Math.sqrt(ve * vb);
    out[i] = denom > 0 ? cov / denom : null;
  }
  return out;
}

export interface MonthlyGrid {
  years: string[];
  /** cells[year][month-1] = return or null (no data). */
  cells: Map<string, (number | null)[]>;
  /** yearly total per year (from the engine's yearly_returns). */
  totals: Map<string, number | null>;
}

/** Pivot the engine's flat monthly list into a year × month grid. */
export function monthlyGrid(monthly: PeriodReturn[], yearly: PeriodReturn[]): MonthlyGrid {
  const cells = new Map<string, (number | null)[]>();
  for (const p of monthly) {
    const [y, m] = p.period.split('-');
    if (!cells.has(y)) cells.set(y, new Array(12).fill(null));
    cells.get(y)![Number(m) - 1] = p.ret;
  }
  return {
    years: [...cells.keys()].sort(),
    cells,
    totals: new Map(yearly.map((p) => [p.period, p.ret])),
  };
}

/**
 * Daily-rebalanced equal-weight basket of every symbol in a close panel,
 * as a synthetic price series starting at 1.0. Used as the fallback benchmark
 * when the sample data ships no index series: for a stock-picking strategy on
 * a fixed universe, "the whole universe, equally weighted" is the natural
 * do-nothing alternative.
 */
export function equalWeightCurve(close: (number | null)[][]): number[] {
  const out: number[] = new Array(close.length).fill(1);
  for (let i = 1; i < close.length; i++) {
    let sum = 0;
    let k = 0;
    for (let c = 0; c < close[i].length; c++) {
      const a = close[i - 1][c];
      const b = close[i][c];
      if (a == null || b == null || !(a > 0)) continue;
      sum += b / a - 1;
      k++;
    }
    out[i] = out[i - 1] * (1 + (k > 0 ? sum / k : 0));
  }
  return out;
}
