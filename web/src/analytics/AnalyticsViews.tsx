// The chat analytics page (#207): the private chats in numbers, when the messages come, and who they are with; and
// the console card that leaves people out of them.
import { useEffect, useMemo, useState } from 'react';
import { ArrowClockwise, ChatCircleDots, Check, CircleNotch, MagnifyingGlass } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { ApiError } from '../api/client';
import { getExcludeCandidates, getExcluded, hourBars, monthBars, setExcluded, typeBars, weekdayBars, type Bar, type Candidate, type Ranked } from '../api/analytics';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import type { AnalyticsState } from './useAnalytics';

const n = (value: number) => value.toLocaleString('zh-CN');
const day = (seconds: number | null) => (seconds ? new Date(seconds * 1000).toLocaleDateString('zh-CN') : '—');
const share = (part: number, whole: number) => (whole ? Math.round((part / whole) * 1000) / 10 : 0) + '%';

/** Bars standing side by side, as tall as their share of the largest; `ticks` names some of them underneath. */
export function Columns({ label, bars, ticks }: { label: string; bars: Bar[]; ticks(index: number, bar: Bar): string | null }) {
  if (!bars.length) return <p className="muted">没有消息。</p>;
  const max = Math.max(1, ...bars.map(b => b.value));
  const peak = bars.reduce((a, b) => (b.value > a.value ? b : a));
  return <figure className="chart">
    <div className="chart-columns" role="img" aria-label={label + '：最多的是' + peak.label + '，' + n(peak.value) + ' 条'}>
      {bars.map(b => <span key={b.label} className="chart-column" title={b.label + ' · ' + n(b.value) + ' 条'}>
        <span style={{ height: (b.value / max) * 100 + '%' }} /></span>)}
    </div>
    <div className="chart-ticks" aria-hidden="true">{bars.map((b, i) => <span key={b.label}>{ticks(i, b)}</span>)}</div>
  </figure>;
}

const january = (b: Bar) => b.label.endsWith(' 1 月');
/**
 * Where a year's name goes under the months: each January, and the first month when no January follows soon, that is
 * within 6 months or an eighth of the chart, as the bars of a long chart are narrow on a phone.
 */
export const yearStarts = (months: Bar[], i: number) =>
  january(months[i]) || (i === 0 && !months.slice(1, Math.max(7, Math.ceil(months.length / 8) + 1)).some(january));

/** One bar a row, each with its count and share of `total`. */
function Shares({ label, bars, total }: { label: string; bars: Bar[]; total: number }) {
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
      <span className="rank-line"><strong>{person.displayName}</strong><span className="num">{n(person.messageCount)} 条</span></span>
      <span className="rank-split" aria-hidden="true"><span style={{ width: width(person.sentCount) }} /><span style={{ width: width(person.receivedCount) }} /></span>
      <span className="cache-text">我发 {n(person.sentCount)} · 对方 {n(person.receivedCount)} · 最近 {day(person.lastMessageTime)}</span>
    </span>
    <button type="button" className="secondary-button" disabled={!canChat} title={canChat ? undefined : '会话列表里没有这个聊天'} aria-label={'查看和' + person.displayName + '的聊天'} onClick={onChat}><ChatCircleDots size={12} />查看聊天</button>
  </li>;
}

export function PrivateAnalytics({ state, onRetry, onRecount, canChat, onChat }: {
  state: AnalyticsState; onRetry(): void; onRecount(): void; canChat(username: string): boolean; onChat(username: string): void;
}) {
  if (state.status === 'error') {
    return <div className="pane-message" role="alert"><strong>读取聊天分析失败</strong><p>{state.message}</p>
      <button type="button" className="secondary-button" onClick={onRetry}>重试</button></div>;
  }
  if (state.status !== 'ready') {
    return <div className="analytics-page" aria-busy="true"><GridSkeleton label="正在统计私聊消息，第一次可能要一会儿" cells={6} /><ListSkeleton label="正在读取聊天排行" rows={5} /></div>;
  }
  const { overall, rankings, spread } = state;
  const months = monthBars(spread);
  const top = rankings[0]?.messageCount ?? 0;
  return <div className="analytics-page">
    <section className="analytics-block" aria-label="总览">
      <div className="analytics-heading"><h2>总览</h2>
        <button type="button" className="secondary-button" title="不用缓存的结果，重新读取所有私聊" onClick={onRecount}><ArrowClockwise size={12} />重新统计</button></div>
      <dl className="info-grid analytics-numbers">
        <div><dt>消息</dt><dd className="num">{n(overall.totalMessages)} 条</dd></div>
        <div><dt>我发出的</dt><dd className="num">{n(overall.sentMessages)} 条 · {share(overall.sentMessages, overall.totalMessages)}</dd></div>
        <div><dt>我收到的</dt><dd className="num">{n(overall.receivedMessages)} 条 · {share(overall.receivedMessages, overall.totalMessages)}</dd></div>
        <div><dt>有消息的月份</dt><dd className="num">{n(Object.values(spread.monthlyDistribution).filter(v => v > 0).length)} 个</dd></div>
        <div><dt>第一条</dt><dd className="num">{day(overall.firstMessageTime)}</dd></div>
        <div><dt>最近一条</dt><dd className="num">{day(overall.lastMessageTime)}</dd></div>
      </dl>
      <p className="card-note">只统计私聊，不含群聊、公众号和「不统计的好友」。</p>
    </section>
    <section className="analytics-block" aria-label="消息类型">
      <h2>消息类型</h2>
      <Shares label="各类消息的条数" bars={typeBars(overall)} total={overall.totalMessages} />
    </section>
    <section className="analytics-block" aria-label="一天中的时段">
      <h2>一天中的时段</h2>
      <Columns label="每小时的消息" bars={hourBars(spread)} ticks={i => (i % 6 === 0 ? String(i) : null)} />
    </section>
    <section className="analytics-block" aria-label="一周中的哪天">
      <h2>一周中的哪天</h2>
      <Columns label="星期几的消息" bars={weekdayBars(spread)} ticks={(_, b) => b.label} />
    </section>
    <section className="analytics-block" aria-label="每个月">
      <h2>每个月</h2>
      <Columns label="每个月的消息" bars={months} ticks={(i, b) => (yearStarts(months, i) ? b.label.slice(0, 4) : null)} />
    </section>
    <section className="analytics-block" aria-label="聊得最多的人">
      <h2>聊得最多的人</h2>
      {rankings.length
        ? <><ol className="rank-list">{rankings.map((person, i) => <RankRow key={person.username} rank={i + 1} person={person} top={top}
          canChat={canChat(person.username)} onChat={() => onChat(person.username)} />)}</ol>
          <p className="card-note"><span className="rank-key rank-key-sent" />我发出的　<span className="rank-key rank-key-received" />对方发来的</p></>
        : <p className="muted">没有私聊消息。</p>}
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
    }, problem => { if (!controller.signal.aborted) setData({ error: (problem instanceof ApiError && problem.message) || '读取失败' }); });
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

  if (!data) return <ListSkeleton label="正在读取私聊" rows={4} />;
  if ('error' in data) return <><p className="card-note setting-warning" role="alert">{data.error}</p><button type="button" className="secondary-button" onClick={() => setRound(r => r + 1)}>重试</button></>;
  const changed = picked.size !== data.saved.length || data.saved.some(u => !picked.has(u));
  const save = async () => {
    setBusy(true); setNote(null);
    try {
      const saved = await setExcluded([...picked]);
      setData({ ...data, saved });
      setPicked(new Set(saved));
      setNote({ text: '已保存，正在按新的名单重新统计。' });
      onSaved();
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || '没有保存，请重试。', error: true });
    } finally {
      setBusy(false);
    }
  };
  return <div className="excluded-people">
    <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
      <input type="search" aria-label="搜索私聊" placeholder="搜索名字或微信号" value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
    <div className="excluded-list" role="group" aria-label="不统计的好友">
      {listed.map(p => <label key={p.username} className="checkbox">
        <input type="checkbox" checked={picked.has(p.username)} onChange={event => {
          const next = new Set(picked);
          if (event.target.checked) next.add(p.username); else next.delete(p.username);
          setPicked(next); setNote(null);
        }} />
        <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{p.displayName}</span></label>)}
      {!shown.length && <p className="card-note">没有找到。</p>}
    </div>
    {shown.length > listed.length && <p className="card-note">还有 {n(shown.length - listed.length)} 个，搜索名字或微信号找到他们。</p>}
    <button type="button" className="primary-button" disabled={!changed || busy} onClick={save}>{busy && <CircleNotch size={14} className="spin" />}保存（不统计 {n(picked.size)} 人）</button>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </div>;
}
