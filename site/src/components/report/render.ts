// Full backtest report: headline strip + four tabs (Profit / Risk / Reward /
// Trades), built as plain DOM from the engine's Report JSON. Everything the
// tabs show is either straight out of the Report or reshaped by ./derive.ts —
// no metric is computed here.

import type { BootstrapCi, Report, Trade } from './types.ts';
import { renderChart } from './charts.ts';
import {
  annualRows,
  drawdownEpisodes,
  isoDate,
  monthlyGrid,
  pctDaysBeating,
  rollingCorrelation,
  tradeHistogram,
  yearlyFromCurve,
} from './derive.ts';

export interface ReportContext {
  /** Display name of the benchmark series ('SPY' or 'EW universe'). */
  benchmarkLabel: string;
}

// --- formatting ------------------------------------------------------------

const esc = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const isNum = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);

const pct = (v: number | null | undefined, dp = 1) => (isNum(v) ? `${(v * 100).toFixed(dp)}%` : '—');
const pctS = (v: number | null | undefined, dp = 1) =>
  isNum(v) ? `${v > 0 ? '+' : ''}${(v * 100).toFixed(dp)}%` : '—';
const num = (v: number | null | undefined, dp = 2) => (isNum(v) ? v.toFixed(dp) : '—');
const int = (v: number | null | undefined) => (isNum(v) ? String(Math.round(v)) : '—');
const days = (v: number | null | undefined) => (isNum(v) ? `${Math.round(v)} d` : '—');
const money = (v: number | null | undefined) => (isNum(v) ? v.toFixed(2) : '—');

const signCls = (v: number | null | undefined) => (isNum(v) ? (v > 0 ? ' pos' : v < 0 ? ' neg' : '') : '');

// --- small builders ----------------------------------------------------------

interface Card {
  label: string;
  value: string;
  /** color the value by this number's sign */
  sign?: number | null;
  sub?: string;
  tip?: string;
}

const card = (c: Card) =>
  `<div class="pg-metric"${c.tip ? ` title="${esc(c.tip)}"` : ''}>` +
  `<div class="k">${c.label}</div><div class="v${signCls(c.sign)}">${c.value}</div>` +
  (c.sub ? `<div class="s">${c.sub}</div>` : '') +
  `</div>`;

const cards = (list: (Card | null)[]) =>
  `<div class="pg-metrics pg-metrics-report">${list.filter(Boolean).map((c) => card(c!)).join('')}</div>`;

const section = (title: string, body: string, note = '') =>
  `<div class="pg-section"><h4 class="pg-section-title">${title}</h4>${body}` +
  (note ? `<p class="pg-note">${note}</p>` : '') +
  `</div>`;

/** Heatmap cell background: green/red, opacity scaled to |ret| (capped at 8%). */
function cellBg(ret: number | null): string {
  if (!isNum(ret) || ret === 0) return '';
  const a = Math.min(Math.abs(ret) / 0.08, 1) * 0.55 + 0.08;
  return ret > 0 ? `background:rgba(34,170,110,${a.toFixed(2)})` : `background:rgba(226,88,98,${a.toFixed(2)})`;
}

// --- tab bodies ------------------------------------------------------------

function monthlyHeatmapHtml(report: Report): string {
  const grid = monthlyGrid(report.monthly_returns, report.yearly_returns);
  if (grid.years.length === 0) return '<p class="pg-note">No calendar data.</p>';
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const head = `<tr><th></th>${months.map((m) => `<th>${m}</th>`).join('')}<th>Year</th></tr>`;
  const rows = grid.years
    .map((y) => {
      const cells = grid.cells
        .get(y)!
        .map((r) => `<td style="${cellBg(r)}">${isNum(r) ? (r * 100).toFixed(1) : ''}</td>`)
        .join('');
      const tot = grid.totals.get(y) ?? null;
      return `<tr><th>${y}</th>${cells}<td class="pg-heat-total${signCls(tot)}" style="${cellBg(tot)}">${
        isNum(tot) ? (tot * 100).toFixed(1) : ''
      }</td></tr>`;
    })
    .join('');
  return `<div class="pg-scroll"><table class="pg-heatmap"><thead>${head}</thead><tbody>${rows}</tbody></table></div>`;
}

function annualBarsHtml(report: Report, benchLabel: string): string {
  const bench = report.benchmark ? yearlyFromCurve(report.dates, report.benchmark) : [];
  const rows = annualRows(report.yearly_returns, bench);
  if (rows.length === 0) return '';
  const all = rows.flatMap((r) => [r.strategy, r.benchmark]).filter(isNum);
  const lo = Math.min(0, ...all);
  const hi = Math.max(0, ...all);
  const span = hi - lo || 1;
  const bar = (v: number | null, cls: string) => {
    if (!isNum(v)) return `<div class="pg-abar ${cls}" style="width:0"></div>`;
    const left = ((Math.min(v, 0) - lo) / span) * 100;
    const width = (Math.abs(v) / span) * 100;
    return `<div class="pg-abar ${cls}" style="left:${left.toFixed(2)}%;width:${width.toFixed(2)}%"></div>`;
  };
  const zero = ((0 - lo) / span) * 100;
  const body = rows
    .map(
      (r) =>
        `<div class="pg-annual-row">` +
        `<span class="pg-annual-year">${r.year}</span>` +
        `<div class="pg-annual-bars"><i class="pg-annual-zero" style="left:${zero.toFixed(2)}%"></i>${bar(
          r.strategy,
          'strat',
        )}${bar(r.benchmark, 'bench')}</div>` +
        `<span class="pg-annual-vals"><b class="${signCls(r.strategy).trim()}">${pctS(r.strategy)}</b>` +
        ` <span>vs ${pctS(r.benchmark)}</span>` +
        ` <em class="${signCls(r.excess).trim()}">${isNum(r.excess) ? `(${pctS(r.excess)})` : ''}</em></span>` +
        `</div>`,
    )
    .join('');
  const won = rows.filter((r) => isNum(r.excess) && r.excess! > 0).length;
  const known = rows.filter((r) => isNum(r.excess)).length;
  const note =
    known > 0
      ? `Beat ${esc(benchLabel)} in <b>${won} of ${known}</b> calendar years.`
      : '';
  return `<div class="pg-annual">${body}</div>${note ? `<p class="pg-note">${note}</p>` : ''}`;
}

function episodesHtml(report: Report): string {
  const eps = drawdownEpisodes(report.dates, report.drawdown);
  if (eps.length === 0) return '<p class="pg-note">The curve never left its running peak — no drawdown episodes.</p>';
  const rows = eps
    .map(
      (e, i) =>
        `<tr><td>${i + 1}</td><td>${isoDate(e.start)}</td><td>${isoDate(e.trough)}</td>` +
        `<td>${e.end != null ? isoDate(e.end) : '<span class="pg-open">not yet</span>'}</td>` +
        `<td class="neg">${pct(e.depth)}</td><td>${e.lengthDays}</td>` +
        `<td>${e.recoveryDays != null ? e.recoveryDays : '—'}</td></tr>`,
    )
    .join('');
  return (
    `<div class="pg-scroll"><table class="pg-table"><thead><tr>` +
    `<th>#</th><th>Peak</th><th>Trough</th><th>Recovered</th><th>Depth</th><th>Days under</th><th>Recovery days</th>` +
    `</tr></thead><tbody>${rows}</tbody></table></div>`
  );
}

function histogramHtml(trades: Trade[]): string {
  const bins = tradeHistogram(trades);
  if (bins.length === 0) return '<p class="pg-note">No trades to plot.</p>';
  const maxCount = Math.max(...bins.map((b) => b.count));
  const bars = bins
    .map((b) => {
      const h = (b.count / maxCount) * 100;
      const mid = (b.x0 + b.x1) / 2;
      const tip = `${pctS(b.x0, 0)} to ${pctS(b.x1, 0)}: ${b.count} trade${b.count === 1 ? '' : 's'}`;
      return `<div class="pg-hist-col" title="${tip}"><i class="${mid >= 0 ? 'pos' : 'neg'}" style="height:${h.toFixed(1)}%"></i></div>`;
    })
    .join('');
  return (
    `<div class="pg-hist">${bars}</div>` +
    `<div class="pg-hist-axis"><span>${pctS(bins[0].x0, 0)}</span><span>per-trade net return</span><span>${pctS(
      bins[bins.length - 1].x1,
      0,
    )}</span></div>`
  );
}

function bootstrapHtml(report: Report): string {
  const b = report.bootstrap;
  if (!b) return '';
  const row = (label: string, ci: BootstrapCi, f: (v: number | null) => string) => {
    const vals = [ci.p05, ci.p50, ci.p95];
    if (!vals.every(isNum)) return '';
    const lo = Math.min(ci.p05!, 0);
    const hi = Math.max(ci.p95!, 0);
    const span = hi - lo || 1;
    const px = (v: number) => (((v - lo) / span) * 100).toFixed(2);
    return (
      `<div class="pg-boot-row"><span class="pg-boot-label">${label}</span>` +
      `<div class="pg-boot-band"><i class="pg-boot-zero" style="left:${px(0)}%"></i>` +
      `<i class="pg-boot-range" style="left:${px(ci.p05!)}%;width:${(((ci.p95! - ci.p05!) / span) * 100).toFixed(2)}%"></i>` +
      `<i class="pg-boot-med" style="left:${px(ci.p50!)}%"></i></div>` +
      `<span class="pg-boot-vals">${f(ci.p05)} · <b>${f(ci.p50)}</b> · ${f(ci.p95)}</span></div>`
    );
  };
  return section(
    'How lucky was this run?',
    `<div class="pg-boot">${row('Sharpe', b.sharpe, (v) => num(v))}${row('CAGR', b.cagr, (v) => pct(v))}${row(
      'Max drawdown',
      b.max_drawdown,
      (v) => pct(v),
    )}</div>`,
    `90% confidence bands from ${b.n_samples} block-bootstrap resamples of the daily returns ` +
      `(blocks of ${b.block_len} days preserve short-range autocorrelation). ` +
      `If the band is wide, the headline number owes a lot to luck.`,
  );
}

function tradesTableHtml(trades: Trade[]): string {
  if (trades.length === 0) return '<p class="pg-note">No trades — the strategy never took a position.</p>';
  return `<div class="pg-scroll"><table class="pg-table pg-trades"><thead><tr>
    <th>Symbol</th><th>Side</th><th>Entry</th><th>Entry px</th><th>Exit</th><th>Exit px</th>
    <th>Days</th><th>Return</th><th>MAE</th><th>MFE</th>
  </tr></thead><tbody></tbody></table></div><div class="pg-pager"></div>`;
}

function fillTradesPage(panel: HTMLElement, trades: Trade[], page: number, perPage: number): void {
  const tbody = panel.querySelector('.pg-trades tbody');
  const pager = panel.querySelector<HTMLElement>('.pg-pager');
  if (!tbody || !pager) return;
  const pages = Math.max(1, Math.ceil(trades.length / perPage));
  const p = Math.max(0, Math.min(page, pages - 1));
  const slice = trades.slice(p * perPage, (p + 1) * perPage);
  tbody.innerHTML = slice
    .map(
      (t) =>
        `<tr><td>${esc(t.symbol)}</td><td>${t.side}</td>` +
        `<td>${isoDate(t.entry_date)}</td><td>${money(t.entry_price)}</td>` +
        `<td>${t.exit_date != null ? isoDate(t.exit_date) : '<span class="pg-open">open</span>'}</td>` +
        `<td>${money(t.exit_price ?? null)}</td><td>${t.period}</td>` +
        `<td class="${signCls(t.ret).trim()}">${pctS(t.ret)}</td>` +
        `<td>${pct(t.mae)}</td><td>${pct(t.mfe)}</td></tr>`,
    )
    .join('');
  pager.innerHTML =
    `<button type="button" class="pg-page-prev" ${p === 0 ? 'disabled' : ''}>‹ prev</button>` +
    `<span>${p * perPage + 1}–${Math.min((p + 1) * perPage, trades.length)} of ${trades.length}</span>` +
    `<button type="button" class="pg-page-next" ${p >= pages - 1 ? 'disabled' : ''}>next ›</button>`;
  pager.querySelector('.pg-page-prev')?.addEventListener('click', () => fillTradesPage(panel, trades, p - 1, perPage));
  pager.querySelector('.pg-page-next')?.addEventListener('click', () => fillTradesPage(panel, trades, p + 1, perPage));
}

// --- main ------------------------------------------------------------------

const TABS: [string, string][] = [
  ['profit', 'Profit'],
  ['risk', 'Risk'],
  ['reward', 'Reward'],
  ['trades', 'Trades'],
];

export function renderReport(mount: HTMLElement, report: Report, ctx: ReportContext): void {
  const m = report.metrics;
  const bench = ctx.benchmarkLabel;
  const hasBench = !!report.benchmark;

  const headline = cards([
    {
      label: 'Total return',
      value: pctS(m.total_return),
      sign: m.total_return,
      sub: isNum(m.benchmark_return) ? `${esc(bench)} ${pctS(m.benchmark_return)}` : undefined,
      tip: 'Final NAV ÷ starting NAV − 1, net of fees',
    },
    { label: 'CAGR', value: pctS(m.cagr), sign: m.cagr, tip: 'Compound annual growth rate' },
    { label: 'Sharpe', value: num(m.sharpe), tip: 'Annualized mean ÷ volatility of daily returns (rf = 0)' },
    { label: 'Max drawdown', value: pct(m.max_drawdown), sign: m.max_drawdown, tip: 'Worst peak-to-trough loss' },
    {
      label: 'Win rate',
      value: pct(m.win_rate),
      sub: isNum(m.num_trades) ? `${int(m.num_trades)} closed trades` : undefined,
      tip: 'Share of closed trades with a positive net return',
    },
    hasBench
      ? {
          label: `vs ${esc(bench)}`,
          value: pctS(m.excess_return),
          sign: m.excess_return,
          tip: 'Total return minus the benchmark’s total return',
        }
      : null,
  ]);

  const tabBar = `<div class="pg-tabs" role="tablist">${TABS.map(
    ([key, label]) =>
      `<button type="button" role="tab" data-tab="${key}" aria-selected="false">${label}</button>`,
  ).join('')}</div>`;

  // --- Profit ---
  const profit =
    cards([
      { label: 'CAGR', value: pctS(m.cagr), sign: m.cagr, tip: 'Compound annual growth rate' },
      hasBench
        ? { label: `${esc(bench)} return`, value: pctS(m.benchmark_return), sign: m.benchmark_return }
        : null,
      hasBench
        ? {
            label: 'Excess return',
            value: pctS(m.excess_return),
            sign: m.excess_return,
            tip: 'Total return minus benchmark total return',
          }
        : null,
      hasBench
        ? { label: 'Alpha (ann.)', value: pctS(m.alpha), sign: m.alpha, tip: 'Annualized CAPM alpha vs the benchmark (rf = 0)' }
        : null,
      hasBench ? { label: 'Beta', value: num(m.beta), tip: 'Sensitivity to benchmark daily moves' } : null,
      m.ytd !== undefined ? { label: 'YTD', value: pctS(m.ytd), sign: m.ytd, tip: 'Return since the last calendar year end' } : null,
      m.one_year !== undefined ? { label: '1 year', value: pctS(m.one_year), sign: m.one_year, tip: 'Trailing 1-year return' } : null,
      m.three_year !== undefined
        ? { label: '3 years', value: pctS(m.three_year), sign: m.three_year, tip: 'Trailing 3-year return' }
        : null,
    ]) +
    section('Growth of $1', `<div data-chart="equity"></div>`) +
    section('Monthly returns (%)', monthlyHeatmapHtml(report)) +
    section('Calendar years vs benchmark', annualBarsHtml(report, bench));

  // --- Risk ---
  const risk =
    cards([
      { label: 'Max drawdown', value: pct(m.max_drawdown), sign: m.max_drawdown, tip: 'Worst peak-to-trough loss' },
      { label: 'Avg drawdown', value: pct(m.avg_drawdown), sign: m.avg_drawdown, tip: 'Mean of the underwater series' },
      { label: 'Ulcer index', value: pct(m.ulcer_index), tip: 'Root-mean-square drawdown — depth and persistence together' },
      { label: 'Longest underwater', value: days(m.max_drawdown_duration), tip: 'Longest run of days below the running peak' },
      { label: 'Ann. volatility', value: pct(m.ann_volatility), tip: 'Std-dev of daily returns × √252' },
      { label: 'VaR 95%', value: pct(m.var_95), sign: m.var_95, tip: '5th percentile of daily returns — a 1-in-20 bad day' },
      { label: 'CVaR 95%', value: pct(m.cvar_95), sign: m.cvar_95, tip: 'Average of the days beyond VaR — how bad the bad days are' },
      { label: 'Best day', value: pctS(m.best_day), sign: m.best_day },
      { label: 'Worst day', value: pctS(m.worst_day), sign: m.worst_day },
      { label: 'Skew', value: num(m.skew), tip: 'Asymmetry of daily returns (negative = crash-prone)' },
      { label: 'Kurtosis', value: num(m.kurtosis), tip: 'Fat-tailedness of daily returns (excess, 0 = normal)' },
      { label: 'Recovery factor', value: num(m.recovery_factor), tip: 'Total return ÷ |max drawdown|' },
    ]) +
    section('Underwater (drawdown from peak)', `<div data-chart="underwater"></div>`) +
    section('Worst drawdowns', episodesHtml(report));

  // --- Reward ---
  const pctBeat = hasBench ? pctDaysBeating(report.equity, report.benchmark!) : null;
  const reward =
    cards([
      { label: 'Sharpe', value: num(m.sharpe), tip: 'Annualized mean ÷ volatility of daily returns (rf = 0)' },
      { label: 'Sortino', value: num(m.sortino), tip: 'Like Sharpe, but only downside volatility counts' },
      { label: 'Calmar', value: num(m.calmar), tip: 'CAGR ÷ |max drawdown|' },
      { label: 'Profit factor', value: num(m.profit_factor), tip: 'Gross trade gains ÷ gross trade losses' },
      hasBench
        ? { label: 'Information ratio', value: num(m.information_ratio), tip: 'Active return ÷ tracking error, annualized' }
        : null,
      hasBench
        ? { label: 'Tracking error', value: pct(m.tracking_error), tip: 'Std-dev of daily active returns, annualized' }
        : null,
      hasBench
        ? {
            label: `Days ≥ ${esc(bench)}`,
            value: pct(pctBeat),
            tip: 'Share of days the rebased equity curve sits at or above the benchmark',
          }
        : null,
      { label: 'Time in market', value: pct(m.time_in_market), tip: 'Days with any position ÷ all days' },
      { label: 'Avg exposure', value: pct(m.avg_exposure), tip: 'Average fraction of capital deployed' },
    ]) +
    section('Rolling 1-year Sharpe', `<div data-chart="rsharpe"></div>`) +
    section('Rolling 1-year volatility', `<div data-chart="rvol"></div>`) +
    (hasBench ? section(`Rolling 1-year correlation to ${esc(bench)}`, `<div data-chart="rcorr"></div>`) : '') +
    bootstrapHtml(report);

  // --- Trades ---
  const trades =
    cards([
      { label: 'Trades', value: int(m.num_trades), tip: 'Closed round trips' },
      { label: 'Win rate', value: pct(m.win_rate) },
      { label: 'Expectancy', value: pctS(m.expectancy), sign: m.expectancy, tip: 'Average net return per closed trade' },
      { label: 'Payoff ratio', value: num(m.payoff_ratio), tip: 'Average win ÷ average loss' },
      { label: 'Avg win', value: pctS(m.avg_win), sign: m.avg_win },
      { label: 'Avg loss', value: pctS(m.avg_loss), sign: m.avg_loss },
      { label: 'Best trade', value: pctS(m.best_trade), sign: m.best_trade },
      { label: 'Worst trade', value: pctS(m.worst_trade), sign: m.worst_trade },
      { label: 'Max consec. losses', value: int(m.max_consecutive_losses) },
      { label: 'Avg holding', value: days(m.avg_holding_period) },
    ]) +
    section('Trade return distribution', histogramHtml(report.trades)) +
    section(
      'All trades',
      tradesTableHtml(report.trades),
      report.trades.length
        ? 'MAE / MFE = worst / best interim return while the position was open. Open trades are marked to market.'
        : '',
    );

  const bodies: Record<string, string> = { profit, risk, reward, trades };
  mount.innerHTML =
    headline +
    tabBar +
    TABS.map(([key]) => `<section class="pg-tabpanel" data-tab="${key}" hidden>${bodies[key]}</section>`).join('');

  // Charts: build now, draw when their tab is (or becomes) visible.
  const pendingDraws = new Map<string, (() => void)[]>();
  const addChart = (tab: string, sel: string, build: (el: HTMLElement) => () => void) => {
    const el = mount.querySelector<HTMLElement>(`[data-chart="${sel}"]`);
    if (!el) return;
    const draw = build(el);
    pendingDraws.set(tab, [...(pendingDraws.get(tab) ?? []), draw]);
  };

  addChart('profit', 'equity', (el) =>
    renderChart(
      el,
      '',
      report.dates,
      [
        { label: 'Strategy', values: report.equity, colorVar: '--pg-line' },
        ...(hasBench ? [{ label: bench, values: report.benchmark!, colorVar: '--pg-line-2' }] : []),
      ],
      { height: 260, baseline: 1, format: (v) => `${v.toFixed(2)}×` },
    ),
  );
  addChart('risk', 'underwater', (el) =>
    renderChart(el, '', report.dates, [{ label: 'Drawdown', values: report.drawdown, colorVar: '--pg-neg', fill: true }], {
      height: 200,
      baseline: 0,
      fillBase: 0,
      format: (v) => `${(v * 100).toFixed(0)}%`,
    }),
  );
  addChart('reward', 'rsharpe', (el) =>
    renderChart(el, '', report.dates, [{ label: 'Sharpe (252d)', values: report.rolling_sharpe, colorVar: '--pg-line' }], {
      height: 180,
      baseline: 0,
      format: (v) => v.toFixed(1),
    }),
  );
  addChart('reward', 'rvol', (el) =>
    renderChart(el, '', report.dates, [{ label: 'Volatility (252d)', values: report.rolling_volatility, colorVar: '--pg-line' }], {
      height: 180,
      include: 0,
      format: (v) => `${(v * 100).toFixed(0)}%`,
    }),
  );
  if (hasBench) {
    addChart('reward', 'rcorr', (el) =>
      renderChart(
        el,
        '',
        report.dates,
        [{ label: 'Correlation (252d)', values: rollingCorrelation(report.equity, report.benchmark!), colorVar: '--pg-line-2' }],
        { height: 180, baseline: 0, include: 1, format: (v) => v.toFixed(2) },
      ),
    );
  }

  // Trades table pagination. (Scope to the panel — the tab button carries the
  // same data-tab attribute.)
  const tradesPanel = mount.querySelector<HTMLElement>('.pg-tabpanel[data-tab="trades"]');
  if (tradesPanel && report.trades.length) fillTradesPage(tradesPanel, report.trades, 0, 15);

  // Tab switching (remember the active tab across re-runs).
  const drawn = new Set<string>();
  const activate = (key: string) => {
    mount.dataset.activeTab = key;
    for (const btn of mount.querySelectorAll<HTMLButtonElement>('.pg-tabs [data-tab]')) {
      btn.setAttribute('aria-selected', String(btn.dataset.tab === key));
    }
    for (const panel of mount.querySelectorAll<HTMLElement>('.pg-tabpanel')) {
      panel.hidden = panel.dataset.tab !== key;
    }
    if (!drawn.has(key)) {
      drawn.add(key);
      for (const draw of pendingDraws.get(key) ?? []) draw();
    }
  };
  mount.querySelector('.pg-tabs')!.addEventListener('click', (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLElement>('[data-tab]');
    if (btn?.dataset.tab) activate(btn.dataset.tab);
  });
  const remembered = mount.dataset.activeTab;
  activate(remembered && TABS.some(([k]) => k === remembered) ? remembered : 'profit');
}
