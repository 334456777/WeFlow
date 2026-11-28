// 聊天分析、群聊分析和双人报告画的柱状图（#207、#208）；样式与其他图表一起在 app.css 里。
import type { Bar } from './api/analytics';
import { n } from './format';
import { t } from './i18n';

/** 并排的柱子，高度按占最大值的比例；`ticks` 在下面给其中一些柱子写上名称。没有柱子时只显示 `没有消息。`。 */
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
 * 年份的名称写在哪些月份下面：每个一月；第一个月后面不久没有一月时，也写在第一个月下面。
 * 「不久」指 6 个月或图表的八分之一以内（取较长的），因为长图表的柱子在手机上很窄。
 */
export const yearStarts = (months: Bar[], i: number) =>
  january(months[i]) || (i === 0 && !months.slice(1, Math.max(7, Math.ceil(months.length / 8) + 1)).some(january));
