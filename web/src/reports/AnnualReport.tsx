// The annual report page (#208): the years with messages to pick from, and one year (or all of them) told in
// numbers: who the chats were with, month by month and hour by hour, the moments that stand out, the words said most.
import type { ReactNode } from 'react';
import { ArrowClockwise } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { busiestHour, formatWait, getAnnualReport, type AnnualReport } from '../api/reports';
import { n } from '../analytics/AnalyticsViews';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';

export const WEEKDAYS = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];

export const Failed = ({ title, message, onRetry }: { title: string; message: string; onRetry(): void }) =>
  <div className="pane-message" role="alert"><strong>{title}</strong><p>{message}</p>
    <button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={12} />重试</button></div>;

/** One moment of the year: what it is, its number, and a line about it. */
export function Moment({ label, value, children }: { label: string; value: ReactNode; children: ReactNode }) {
  return <li className="report-moment"><span className="report-label">{label}</span><strong className="report-value num">{value}</strong><span className="report-note">{children}</span></li>;
}

/** Messages by weekday and hour, darker for more. */
export function Heatmap({ data }: { data: number[][] }) {
  const max = Math.max(1, ...data.flat());
  const best = busiestHour(data);
  return <figure className="chart report-heatmap">
    <div className="heatmap-grid" role="img" aria-label={best ? '最常聊天的是' + WEEKDAYS[best.weekday] + ' ' + best.hour + ' 点，' + n(best.count) + ' 条' : '没有消息'}>
      {data.map((row, w) => <div key={w} className="heatmap-row">
        <span className="heatmap-day" aria-hidden="true">{WEEKDAYS[w].slice(1)}</span>
        {row.map((count, h) => <span key={h} className="heatmap-cell" title={WEEKDAYS[w] + ' ' + h + ' 点 · ' + n(count) + ' 条'}
          style={{ opacity: count ? 0.15 + (count / max) * 0.85 : 0.04 }} />)}
      </div>)}
    </div>
    <div className="chart-ticks heatmap-ticks" aria-hidden="true"><span />{Array.from({ length: 24 }, (_, h) => <span key={h}>{h % 6 === 0 ? h : null}</span>)}</div>
  </figure>;
}

/** The words said most, larger the more often. */
export function Phrases({ phrases, label = '常用语' }: { phrases: AnnualReport['topPhrases']; label?: string }) {
  const max = Math.max(1, ...phrases.map(p => p.count));
  return <ul className="word-chips report-phrases" aria-label={label}>
    {phrases.map(p => <li key={p.phrase} style={{ fontSize: 'calc(var(--text-sm) + ' + Math.round((p.count / max) * 8) + 'px)' }}>
      <span className="clamp-1">{p.phrase}</span><small className="num">{n(p.count)}</small></li>)}
  </ul>;
}

export function AnnualReportView({ report }: { report: AnnualReport }) {
  const r = report;
  const span = r.year ? r.year + ' 年' : '这些年';
  const months = r.monthlyTopFriends.filter(m => m.messageCount > 0);
  const top = r.coreFriends[0]?.messageCount ?? 0;
  const best = busiestHour(r.activityHeatmap.data);
  return <article className="analytics-page report" aria-label={(r.year ? r.year + ' 年' : '全部年份') + '的年度报告'}>
    <section className="report-hero" aria-label="总览">
      <span className="report-label">{span}，你在私聊里发出和收到了</span>
      <strong className="report-big num">{n(r.totalMessages)}</strong>
      <span className="report-note">条消息，和 {n(r.totalFriends)} 位好友聊过天。</span>
    </section>

    {r.coreFriends.length > 0 && <section className="analytics-block" aria-label="聊得最多的人">
      <h2>聊得最多的人</h2>
      <ol className="rank-list">{r.coreFriends.map((f, i) => <li key={f.username} className="rank-row">
        <span className="rank-number num" aria-hidden="true">{i + 1}</span>
        <Avatar name={f.displayName} url={f.avatarUrl} />
        <span className="rank-copy">
          <span className="rank-line"><strong>{f.displayName}</strong><span className="num">{n(f.messageCount)} 条</span></span>
          <span className="rank-split" aria-hidden="true"><span style={{ width: (top ? (f.sentCount / top) * 100 : 0) + '%' }} /><span style={{ width: (top ? (f.receivedCount / top) * 100 : 0) + '%' }} /></span>
          <span className="cache-text">我发 {n(f.sentCount)} · 对方 {n(f.receivedCount)}</span>
        </span>
      </li>)}</ol>
    </section>}

    {r.year > 0 && months.length > 0 && <section className="analytics-block" aria-label="每个月">
      <h2>每个月聊得最多的人</h2>
      <ol className="report-months">{r.monthlyTopFriends.map(m => <li key={m.month} className={m.messageCount ? undefined : 'empty'}>
        <span className="report-label">{m.month} 月</span>
        {m.messageCount ? <><strong className="clamp-1">{m.displayName}</strong><span className="report-note num">{n(m.messageCount)} 条</span></> : <span className="report-note">没有聊天</span>}
      </li>)}</ol>
    </section>}

    <section className="analytics-block" aria-label="什么时候聊天">
      <h2>什么时候聊天</h2>
      <Heatmap data={r.activityHeatmap.data} />
      {best && <p className="card-note">最常在{WEEKDAYS[best.weekday]} {best.hour} 点聊天。</p>}
    </section>

    <section className="analytics-block" aria-label="值得记住的">
      <h2>值得记住的</h2>
      <ul className="report-moments">
        {r.peakDay && <Moment label="最热闹的一天" value={r.peakDay.date}>
          这一天有 {n(r.peakDay.messageCount)} 条消息{r.peakDay.topFriend && <>，其中 {n(r.peakDay.topFriendCount)} 条和{r.peakDay.topFriend}</>}。</Moment>}
        {r.longestStreak && <Moment label="连续聊天最久" value={n(r.longestStreak.days) + ' 天'}>
          和{r.longestStreak.friendName}，{r.longestStreak.startDate} 到 {r.longestStreak.endDate}，每天都有消息。</Moment>}
        {r.midnightKing && <Moment label="深夜聊得最多" value={r.midnightKing.displayName}>
          0 点到 6 点的私聊里，{n(r.midnightKing.count)} 条和 TA，占 {r.midnightKing.percentage}%。</Moment>}
        {r.mutualFriend && <Moment label="有来有往" value={r.mutualFriend.displayName}>
          你发 {n(r.mutualFriend.sentCount)} 条，TA 发 {n(r.mutualFriend.receivedCount)} 条，是收发最接近的一位。</Moment>}
        {r.socialInitiative && <Moment label="由你开场" value={r.socialInitiative.initiativeRate + '%'}>
          {n(r.socialInitiative.initiatedChats)} 次对话由你开始，{n(r.socialInitiative.receivedChats)} 次由对方开始{r.socialInitiative.topInitiatedFriend && <>；你最常先找{r.socialInitiative.topInitiatedFriend}，{n(r.socialInitiative.topInitiatedCount ?? 0)} 次</>}。</Moment>}
        {r.responseSpeed && <Moment label="平均回复" value={formatWait(r.responseSpeed.avgResponseTime)}>
          回得最快的是{r.responseSpeed.fastestFriend}，平均 {formatWait(r.responseSpeed.fastestTime)}。</Moment>}
        {r.lostFriend && <Moment label="渐渐少聊的人" value={r.lostFriend.displayName}>
          {r.lostFriend.periodDesc}聊了 {n(r.lostFriend.earlyCount)} 条，之后只有 {n(r.lostFriend.lateCount)} 条。</Moment>}
      </ul>
    </section>

    {r.topPhrases.length > 0 && <section className="analytics-block" aria-label="常说的话">
      <h2>常说的话</h2>
      <Phrases phrases={r.topPhrases} />
    </section>}

    {r.snsStats && r.snsStats.totalPosts > 0 && <section className="analytics-block" aria-label="朋友圈">
      <h2>朋友圈 <small className="muted-num num">发了 {n(r.snsStats.totalPosts)} 条</small></h2>
      <div className="report-likes">
        {r.snsStats.topLikers.length > 0 && <div><span className="report-label">给你点赞最多</span>
          <ul className="word-chips">{r.snsStats.topLikers.map(p => <li key={p.username ?? p.displayName}>{p.displayName}<small className="num">{n(p.count)}</small></li>)}</ul></div>}
        {r.snsStats.topLiked.length > 0 && <div><span className="report-label">你点赞最多</span>
          <ul className="word-chips">{r.snsStats.topLiked.map(p => <li key={p.username ?? p.displayName}>{p.displayName}<small className="num">{n(p.count)}</small></li>)}</ul></div>}
      </div>
    </section>}
  </article>;
}

/** The report of the year picked, 0 for all of them. */
export function AnnualReportPage({ year }: { year: number }) {
  const [report, retry] = useRead('report:' + year, signal => getAnnualReport(year, signal));
  return report.status === 'error'
    ? <Failed title="生成报告失败" message={report.message} onRetry={retry} />
    : report.status === 'loading'
      ? <div className="analytics-page" aria-busy="true"><GridSkeleton label={'正在生成' + (year ? year + ' 年' : '全部年份') + '的报告'} cells={3} /><ListSkeleton label="正在统计" rows={3} /></div>
      : <AnnualReportView report={report.value} />;
}
