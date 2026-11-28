// 年度报告（#208）：用数字讲述一年（或者全部年份）的私聊：和谁聊、每个月和每个时段的情况、值得记住的时刻，
// 以及最常说的话。选年份在 ReportsPage.tsx。
import type { ReactNode } from 'react';
import { Avatar } from '../Avatar';
import { busiestHour, formatWait, getAnnualReport, type AnnualReport } from '../api/reports';
import { monthName } from '../api/analytics';
import { Failed } from '../Failed';
import { n } from '../format';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';
import { weekdays } from '../calendar/dates';
import { useShownReport } from './shown';
import { t } from '../i18n';

/** 报告里值得记住的一项：名称、数字和一句说明。 */
export function Moment({ label, value, children }: { label: string; value: ReactNode; children: ReactNode }) {
  return <li className="report-moment"><span className="report-label">{label}</span><strong className="report-value num">{value}</strong><span className="report-note">{children}</span></li>;
}

/** 按星期和小时分布的消息，消息越多颜色越深。 */
export function Heatmap({ data }: { data: number[][] }) {
  const max = Math.max(1, ...data.flat());
  const best = busiestHour(data);
  const days = weekdays();
  return <figure className="chart report-heatmap">
    <div className="heatmap-grid" role="img" aria-label={best ? t('最常聊天的是{day} {hour} 点，{n} 条', { day: days[best.weekday].short, hour: best.hour, n: n(best.count) }) : t('没有消息')}>
      {data.map((row, w) => <div key={w} className="heatmap-row">
        <span className="heatmap-day" aria-hidden="true">{days[w].narrow}</span>
        {row.map((count, h) => <span key={h} className="heatmap-cell" title={t('{day} {hour} 点 · {n} 条', { day: days[w].short, hour: h, n: n(count) })}
          style={{ opacity: count ? 0.15 + (count / max) * 0.85 : 0.04 }} />)}
      </div>)}
    </div>
    <div className="chart-ticks heatmap-ticks" aria-hidden="true"><span />{Array.from({ length: 24 }, (_, h) => <span key={h}>{h % 6 === 0 ? h : null}</span>)}</div>
  </figure>;
}

/** 最常说的话，说得越多字越大。 */
export function Phrases({ phrases, label = t('常用语') }: { phrases: AnnualReport['topPhrases']; label?: string }) {
  const max = Math.max(1, ...phrases.map(p => p.count));
  return <ul className="word-chips report-phrases" aria-label={label}>
    {phrases.map(p => <li key={p.phrase} style={{ fontSize: 'calc(var(--text-sm) + ' + Math.round((p.count / max) * 8) + 'px)' }}>
      <span className="clamp-1">{p.phrase}</span><small className="num">{n(p.count)}</small></li>)}
  </ul>;
}

/** 年度报告的各个部分，从总览到朋友圈。 */
export function AnnualReportView({ report }: { report: AnnualReport }) {
  const r = report;
  const span = r.year ? t('{year} 年', { year: r.year }) : t('这些年');
  const months = r.monthlyTopFriends.filter(m => m.messageCount > 0);
  const top = r.coreFriends[0]?.messageCount ?? 0;
  const best = busiestHour(r.activityHeatmap.data);
  return <article className="analytics-page report" aria-label={r.year ? t('{year} 年的年度报告', { year: r.year }) : t('全部年份的年度报告')}>
    <section className="report-hero" aria-label={t('总览')}>
      <span className="report-label">{t('{span}，你在私聊里发出和收到了', { span })}</span>
      <strong className="report-big num">{n(r.totalMessages)}</strong>
      <span className="report-note">{t('条消息，和 {totalFriends} 位好友聊过天。', { totalFriends: n(r.totalFriends) })}</span>
    </section>

    {r.coreFriends.length > 0 && <section className="analytics-block" aria-label={t('聊得最多的人')}>
      <h2>{t('聊得最多的人')}</h2>
      <ol className="rank-list">{r.coreFriends.map((f, i) => <li key={f.username} className="rank-row">
        <span className="rank-number num" aria-hidden="true">{i + 1}</span>
        <Avatar name={f.displayName} url={f.avatarUrl} />
        <span className="rank-copy">
          <span className="rank-line"><strong>{f.displayName}</strong><span className="num">{t('{messageCount} 条', { messageCount: n(f.messageCount) })}</span></span>
          <span className="rank-split" aria-hidden="true"><span style={{ width: (top ? (f.sentCount / top) * 100 : 0) + '%' }} /><span style={{ width: (top ? (f.receivedCount / top) * 100 : 0) + '%' }} /></span>
          <span className="cache-text">{t('我发 {sentCount} · 对方 {receivedCount}', { sentCount: n(f.sentCount), receivedCount: n(f.receivedCount) })}</span>
        </span>
      </li>)}</ol>
    </section>}

    {r.year > 0 && months.length > 0 && <section className="analytics-block" aria-label={t('每个月')}>
      <h2>{t('每个月聊得最多的人')}</h2>
      <ol className="report-months">{r.monthlyTopFriends.map(m => <li key={m.month} className={m.messageCount ? undefined : 'empty'}>
        <span className="report-label">{monthName(m.month)}</span>
        {m.messageCount ? <><strong className="clamp-1">{m.displayName}</strong><span className="report-note num">{t('{messageCount} 条', { messageCount: n(m.messageCount) })}</span></> : <span className="report-note">{t('没有聊天')}</span>}
      </li>)}</ol>
    </section>}

    <section className="analytics-block" aria-label={t('什么时候聊天')}>
      <h2>{t('什么时候聊天')}</h2>
      <Heatmap data={r.activityHeatmap.data} />
      {best && <p className="card-note">{t('最常在{day} {hour} 点聊天。', { day: weekdays()[best.weekday].short, hour: best.hour })}</p>}
    </section>

    <section className="analytics-block" aria-label={t('值得记住的')}>
      <h2>{t('值得记住的')}</h2>
      <ul className="report-moments">
        {r.peakDay && <Moment label={t('最热闹的一天')} value={r.peakDay.date}>
          {r.peakDay.topFriend
            ? t('这一天有 {n} 条消息，其中 {friendCount} 条和{friend}。', { n: n(r.peakDay.messageCount), friendCount: n(r.peakDay.topFriendCount), friend: r.peakDay.topFriend })
            : t('这一天有 {n} 条消息。', { n: n(r.peakDay.messageCount) })}</Moment>}
        {r.longestStreak && <Moment label={t('连续聊天最久')} value={t('{n} 天', { n: n(r.longestStreak.days) })}>
          {t('和{friendName}，{startDate} 到 {endDate}，每天都有消息。', { friendName: r.longestStreak.friendName, startDate: r.longestStreak.startDate, endDate: r.longestStreak.endDate })}</Moment>}
        {r.midnightKing && <Moment label={t('深夜聊得最多')} value={r.midnightKing.displayName}>
          {t('0 点到 6 点的私聊里，{count} 条和 TA，占 {percentage}%。', { count: n(r.midnightKing.count), percentage: r.midnightKing.percentage })}</Moment>}
        {r.mutualFriend && <Moment label={t('有来有往')} value={r.mutualFriend.displayName}>
          {t('你发 {sentCount} 条，TA 发 {receivedCount} 条，是收发最接近的一位。', { sentCount: n(r.mutualFriend.sentCount), receivedCount: n(r.mutualFriend.receivedCount) })}</Moment>}
        {r.socialInitiative && <Moment label={t('由你开场')} value={r.socialInitiative.initiativeRate + '%'}>
          {r.socialInitiative.topInitiatedFriend
            ? t('{initiated} 次对话由你开始，{received} 次由对方开始；你最常先找{friend}，{n} 次。', { initiated: n(r.socialInitiative.initiatedChats), received: n(r.socialInitiative.receivedChats), friend: r.socialInitiative.topInitiatedFriend, n: n(r.socialInitiative.topInitiatedCount ?? 0) })
            : t('{initiated} 次对话由你开始，{received} 次由对方开始。', { initiated: n(r.socialInitiative.initiatedChats), received: n(r.socialInitiative.receivedChats) })}</Moment>}
        {r.responseSpeed && <Moment label={t('平均回复')} value={formatWait(r.responseSpeed.avgResponseTime)}>
          {t('回得最快的是{fastestFriend}，平均 {fastestTime}。', { fastestFriend: r.responseSpeed.fastestFriend, fastestTime: formatWait(r.responseSpeed.fastestTime) })}</Moment>}
        {r.lostFriend && <Moment label={t('渐渐少聊的人')} value={r.lostFriend.displayName}>
          {t('{periodDesc}聊了 {earlyCount} 条，之后只有 {lateCount} 条。', { periodDesc: r.lostFriend.periodDesc, earlyCount: n(r.lostFriend.earlyCount), lateCount: n(r.lostFriend.lateCount) })}</Moment>}
      </ul>
    </section>

    {r.topPhrases.length > 0 && <section className="analytics-block" aria-label={t('常说的话')}>
      <h2>{t('常说的话')}</h2>
      <Phrases phrases={r.topPhrases} />
    </section>}

    {r.snsStats && r.snsStats.totalPosts > 0 && <section className="analytics-block" aria-label={t('朋友圈')}>
      <h2>{t('朋友圈')} <small className="muted-num num">{t('发了 {totalPosts} 条', { totalPosts: n(r.snsStats.totalPosts) })}</small></h2>
      <div className="report-likes">
        {r.snsStats.topLikers.length > 0 && <div><span className="report-label">{t('给你点赞最多')}</span>
          <ul className="word-chips">{r.snsStats.topLikers.map(p => <li key={p.username ?? p.displayName}>{p.displayName}<small className="num">{n(p.count)}</small></li>)}</ul></div>}
        {r.snsStats.topLiked.length > 0 && <div><span className="report-label">{t('你点赞最多')}</span>
          <ul className="word-chips">{r.snsStats.topLiked.map(p => <li key={p.username ?? p.displayName}>{p.displayName}<small className="num">{n(p.count)}</small></li>)}</ul></div>}
      </div>
    </section>}
  </article>;
}

/** 选中的那一年的报告，`year` 为 0 时是全部年份；`onShown` 收到当前显示的是哪一份报告。 */
export function AnnualReportPage({ year, onShown }: { year: number; onShown(key: string | null): void }) {
  const [report, retry] = useRead('report:' + year, signal => getAnnualReport(year, signal));
  useShownReport(report.status === 'ready' ? 'annual:' + report.value.year : null, onShown);
  return report.status === 'error'
    ? <Failed title={t('生成报告失败')} message={report.message} onRetry={retry} />
    : report.status === 'loading'
      ? <div className="analytics-page" aria-busy="true"><GridSkeleton label={year ? t('正在生成{year} 年的报告', { year }) : t('正在生成全部年份的报告')} cells={3} /><ListSkeleton label={t('正在统计')} rows={3} /></div>
      : <AnnualReportView report={report.value} />;
}
