// The chat analytics page (#207): the private chats in numbers, when the messages come, and who they are with; and
// the console card that leaves people out of them.
import { useEffect, useMemo, useState } from 'react';
import { ArrowClockwise, ChatCircleDots, Check, CircleNotch, MagnifyingGlass } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { ApiError } from '../api/client';
import { getExcludeCandidates, getExcluded, hourBars, monthBars, setExcluded, typeBars, weekdayBars, type Bar, type Candidate, type Ranked } from '../api/analytics';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import type { AnalyticsState } from './useAnalytics';
import { locale, t } from '../i18n';

export const n = (value: number) => value.toLocaleString(locale());
export const day = (seconds: number | null) => (seconds ? new Date(seconds * 1000).toLocaleDateString(locale()) : '—');
export const share = (part: number, whole: number) => (whole ? Math.round((part / whole) * 1000) / 10 : 0) + '%';

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

/** One bar a row, each with its count and share of `total`. */
export function Shares({ label, bars, total }: { label: string; bars: Bar[]; total: number }) {
  return <ul className="chart-shares" aria-label={label}>
    {bars.map(b => <li key={b.label}>
      <span className="chart-share-label">{b.label}</span>
      <span className="job-bar" aria-hidden="true"><span style={{ width: (total ? (b.value / total) * 100 : 0) + '%' }} /></span>
      <span className="num">{n(b.value)}</span><span className="num muted-num">{share(b.value, total)}</span>
    </li>)}
  </ul>;
}

function RankRow({ rank, person, top, canChat, onChat }: { rank: number; person: Ranked; top: number; canChat: boolean; onChat(): void }) {
  const width = (part: number) => (top ? (part / top) * 100 : 0) + '%';
  return <li className="rank-row">
    <span className="rank-number num" aria-hidden="true">{rank}</span>
    <Avatar name={person.displayName} url={person.avatarUrl} />
    <span className="rank-copy">
      <span className="rank-line"><strong>{person.displayName}</strong><span className="num">{t('{messageCount} 条', { messageCount: n(person.messageCount) })}</span></span>
      <span className="rank-split" aria-hidden="true"><span style={{ width: width(person.sentCount) }} /><span style={{ width: width(person.receivedCount) }} /></span>
      <span className="cache-text">{t('我发 {sentCount} · 对方 {receivedCount} · 最近 {lastMessageTime}', { sentCount: n(person.sentCount), receivedCount: n(person.receivedCount), lastMessageTime: day(person.lastMessageTime) })}</span>
    </span>
    <button type="button" className="secondary-button rank-wide" disabled={!canChat} title={canChat ? undefined : t('会话列表里没有这个聊天')} aria-label={t('查看和{name}的聊天', { name: person.displayName })} onClick={onChat}><ChatCircleDots size={12} />{t('查看聊天')}</button>
  </li>;
}

export function PrivateAnalytics({ state, onReload, canChat, onChat }: {
  state: AnalyticsState; onReload(): void; canChat(username: string): boolean; onChat(username: string): void;
}) {
  if (state.status === 'error') {
    return <div className="pane-message" role="alert"><strong>{t('读取聊天分析失败')}</strong><p>{state.message}</p>
      <button type="button" className="secondary-button" onClick={onReload}>{t('重试')}</button></div>;
  }
  if (state.status !== 'ready') {
    return <div className="analytics-page" aria-busy="true"><GridSkeleton label={t('正在统计私聊消息，第一次可能要一会儿')} cells={6} /><ListSkeleton label={t('正在读取聊天排行')} rows={5} /></div>;
  }
  const { overall, rankings, spread } = state;
  const months = monthBars(spread);
  const top = rankings[0]?.messageCount ?? 0;
  return <div className="analytics-page">
    <section className="analytics-block" aria-label={t('总览')}>
      <div className="analytics-heading"><h2>{t('总览')}</h2>
        <button type="button" className="secondary-button" title={t('重新读取所有私聊，算上新收到的消息')} onClick={onReload}><ArrowClockwise size={12} />{t('重新统计')}</button></div>
      <dl className="info-grid analytics-numbers">
        <div><dt>{t('消息')}</dt><dd className="num">{t('{totalMessages} 条', { totalMessages: n(overall.totalMessages) })}</dd></div>
        <div><dt>{t('我发出的')}</dt><dd className="num">{t('{sentMessages} 条 · {totalMessages}', { sentMessages: n(overall.sentMessages), totalMessages: share(overall.sentMessages, overall.totalMessages) })}</dd></div>
        <div><dt>{t('我收到的')}</dt><dd className="num">{t('{receivedMessages} 条 · {totalMessages}', { receivedMessages: n(overall.receivedMessages), totalMessages: share(overall.receivedMessages, overall.totalMessages) })}</dd></div>
        <div><dt>{t('有消息的月份')}</dt><dd className="num">{t('{length} 个', { length: n(Object.values(spread.monthlyDistribution).filter(v => v > 0).length) })}</dd></div>
        <div><dt>{t('第一条')}</dt><dd className="num">{day(overall.firstMessageTime)}</dd></div>
        <div><dt>{t('最近一条')}</dt><dd className="num">{day(overall.lastMessageTime)}</dd></div>
      </dl>
      <p className="card-note">{t('只统计私聊，不含群聊、公众号和「不统计的好友」。')}</p>
    </section>
    <section className="analytics-block" aria-label={t('消息类型')}>
      <h2>{t('消息类型')}</h2>
      <Shares label={t('各类消息的条数')} bars={typeBars(overall)} total={overall.totalMessages} />
    </section>
    <section className="analytics-block" aria-label={t('一天中的时段')}>
      <h2>{t('一天中的时段')}</h2>
      <Columns label={t('每小时的消息')} bars={hourBars(spread.hourlyDistribution)} ticks={i => (i % 6 === 0 ? String(i) : null)} />
    </section>
    <section className="analytics-block" aria-label={t('一周中的哪天')}>
      <h2>{t('一周中的哪天')}</h2>
      <Columns label={t('星期几的消息')} bars={weekdayBars(spread)} ticks={(_, b) => b.label} />
    </section>
    <section className="analytics-block" aria-label={t('每个月')}>
      <h2>{t('每个月')}</h2>
      <Columns label={t('每个月的消息')} bars={months} ticks={(i, b) => (yearStarts(months, i) ? String(b.year) : null)} />
    </section>
    <section className="analytics-block" aria-label={t('聊得最多的人')}>
      <h2>{t('聊得最多的人')}</h2>
      {rankings.length
        ? <><ol className="rank-list">{rankings.map((person, i) => <RankRow key={person.username} rank={i + 1} person={person} top={top}
          canChat={canChat(person.username)} onChat={() => onChat(person.username)} />)}</ol>
          <p className="card-note"><span className="rank-key rank-key-sent" />{t('我发出的')}　<span className="rank-key rank-key-received" />{t('对方发来的')}</p></>
        : <p className="muted">{t('没有私聊消息。')}</p>}
    </section>
  </div>;
}

/** The people left out of the analytics: every private chat can be, and those left out already come first. */
export function ExcludedPeople({ onSaved }: { onSaved(): void }) {
  const [data, setData] = useState<{ saved: string[]; people: Candidate[] } | { error: string } | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [keyword, setKeyword] = useState('');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const [round, setRound] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setData(null);
    Promise.all([getExcluded(controller.signal), getExcludeCandidates(controller.signal)]).then(([saved, people]) => {
      setData({ saved, people });
      setPicked(new Set(saved));
    }, problem => { if (!controller.signal.aborted) setData({ error: (problem instanceof ApiError && problem.message) || t('读取失败') }); });
    return () => controller.abort();
  }, [round]);
  const shown = useMemo(() => {
    if (!data || 'error' in data) return [];
    const words = keyword.trim().toLowerCase();
    const first = new Set(data.saved);
    return data.people
      .filter(p => !words || p.displayName.toLowerCase().includes(words) || p.username.toLowerCase().includes(words) || (p.wechatId ?? '').toLowerCase().includes(words))
      .sort((a, b) => Number(first.has(b.username)) - Number(first.has(a.username)));
  }, [data, keyword]);
  // everyone left out (they come first), and a few of the others: the rest are found by name
  const out = data && !('error' in data) ? shown.filter(p => data.saved.includes(p.username)).length : 0;
  const listed = shown.slice(0, out + 8);

  if (!data) return <ListSkeleton label={t('正在读取私聊')} rows={4} />;
  if ('error' in data) return <><p className="card-note setting-warning" role="alert">{data.error}</p><button type="button" className="secondary-button" onClick={() => setRound(r => r + 1)}>{t('重试')}</button></>;
  const changed = picked.size !== data.saved.length || data.saved.some(u => !picked.has(u));
  const save = async () => {
    setBusy(true); setNote(null);
    try {
      const saved = await setExcluded([...picked]);
      setData({ ...data, saved });
      setPicked(new Set(saved));
      setNote({ text: t('已保存，正在按新的名单重新统计。') });
      onSaved();
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || t('没有保存，请重试。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <div className="excluded-people">
    <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
      <input type="search" aria-label={t('搜索私聊')} placeholder={t('搜索名字或微信号')} value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
    <div className="excluded-list" role="group" aria-label={t('不统计的好友')}>
      {listed.map(p => <label key={p.username} className="checkbox">
        <input type="checkbox" checked={picked.has(p.username)} onChange={event => {
          const next = new Set(picked);
          if (event.target.checked) next.add(p.username); else next.delete(p.username);
          setPicked(next); setNote(null);
        }} />
        <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{p.displayName}</span></label>)}
      {!shown.length && <p className="card-note">{t('没有找到。')}</p>}
    </div>
    {shown.length > listed.length && <p className="card-note">{t('还有 {n} 个，搜索名字或微信号找到他们。', { n: n(shown.length - listed.length) })}</p>}
    <button type="button" className="primary-button" disabled={!changed || busy} onClick={save}>{busy && <CircleNotch size={14} className="spin" />}{t('保存（不统计 {n} 人）', { n: n(picked.size) })}</button>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </div>;
}
