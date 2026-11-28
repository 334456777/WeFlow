// The column chart the analytics, the group analytics and the dual report draw (#207, #208); its styles are with the
// other charts in app.css.
import type { Bar } from './api/analytics';
import { n } from './format';
import { t } from './i18n';

/** Bars standing side by side, as tall as their share of the largest; `ticks` names some of them underneath. */
export function Columns({ label, bars, ticks }: { label: string; bars: Bar[]; ticks(index: number, bar: Bar): string | null }) {
  if (!bars.length) return <p className="muted">{t('没有消息。')}</p>;
  const max = Math.max(1, ...bars.map(b => b.value));
  const peak = bars.reduce((a, b) => (b.value > a.value ? b : a));
  return <figure className="chart">
    <div className="chart-columns" role="img" aria-label={t('{label}：最多的是{peak}，{n} 条', { label, peak: peak.label, n: n(peak.value) })}>
      {bars.map(b => <span key={b.label} className="chart-column" title={b.label + ' · ' + t('{n} 条', { n: n(b.value) })}>
        <span style={{ height: (b.value / max) * 100 + '%' }} /></span>)}
    </div>
    <div className="chart-ticks" aria-hidden="true">{bars.map((b, i) => <span key={b.label}>{ticks(i, b)}</span>)}</div>
  </figure>;
}

const january = (b: Bar) => b.month === 1;
/**
 * Where a year's name goes under the months: each January, and the first month when no January follows soon, that is
 * within 6 months or an eighth of the chart, as the bars of a long chart are narrow on a phone.
 */
export const yearStarts = (months: Bar[], i: number) =>
  january(months[i]) || (i === 0 && !months.slice(1, Math.max(7, Math.ceil(months.length / 8) + 1)).some(january));
