// The dual report (#208): me and one friend over a year or all of them — how it began, how much and when we talk,
// who opens and how fast replies come, and the words each of us says.
import { useMemo, useRef, useState } from 'react';
import { CaretDown, MagnifyingGlass, UserCircle } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { getExcludeCandidates, getRankings, monthBars, type Candidate } from '../api/analytics';
import { formatWait, getDualReport, messageText, type DualMessage, type DualReport } from '../api/reports';
import { Columns, n, yearStarts } from '../analytics/AnalyticsViews';
import { Popover } from '../Popover';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';
import { Failed, Heatmap, Moment, Phrases } from './AnnualReport';

/** Up to this many people are listed; the rest are found by name. */
const LISTED = 50;

export interface Friend { username: string; displayName: string; avatarUrl?: string }

/** The friend of the report, picked from the private chats: those talked to most first, the others by name. */
export function FriendPicker({ selected, onSelect }: { selected: Friend | null; onSelect(friend: Friend): void }) {
  const [open, setOpen] = useState(false);
  const [keyword, setKeyword] = useState('');
  const anchor = useRef<HTMLButtonElement>(null);
  // read once it is first opened: the twenty talked to most, then everyone else
  const [asked, setAsked] = useState(false);
  const [people, retry] = useRead('friends:' + asked, async signal => {
    if (!asked) return [];
    const [ranked, all] = await Promise.all([getRankings(20, signal), getExcludeCandidates(signal)]);
    const first = ranked.map<Candidate>(r => ({ username: r.username, displayName: r.displayName, avatarUrl: r.avatarUrl }));
    const seen = new Set(first.map(p => p.username));
    return [...first, ...all.filter(p => !seen.has(p.username))];
  });
  const found = useMemo(() => {
    const list = people.status === 'ready' ? people.value : [];
    const words = keyword.trim().toLowerCase();
    return words ? list.filter(p => p.displayName.toLowerCase().includes(words) || (p.wechatId ?? '').toLowerCase().includes(words)) : list;
  }, [people, keyword]);
  const close = (focus: boolean) => { if (focus) anchor.current?.focus(); setOpen(false); };
  return <>
    <button ref={anchor} type="button" className="group-picker" aria-haspopup="dialog" aria-expanded={open} onClick={() => { setAsked(true); setOpen(!open); }}>
      {selected ? <Avatar name={selected.displayName} url={selected.avatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true"><UserCircle size={20} /></span>}
      <span className="rank-copy"><strong className="clamp-1">{selected?.displayName ?? '选择一位好友'}</strong>
        <span className="cache-text">{selected ? '点这里换一位' : '和 TA 的聊天会生成一份双人报告'}</span></span>
      <CaretDown size={14} aria-hidden="true" />
    </button>
    <Popover anchor={anchor} open={open} onClose={reason => close(reason === 'escape')} label="选择好友" className="group-popover">
      <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
        <input type="search" autoFocus aria-label="搜索好友" placeholder="搜索名字或微信号" value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
      {people.status === 'error'
        ? <><p className="card-note setting-warning" role="alert">{people.message}</p><button type="button" className="secondary-button" onClick={retry}>重试</button></>
        : people.status === 'loading' ? <ListSkeleton label="正在读取私聊" rows={4} />
          : <div className="group-options" role="listbox" data-roving="vertical" data-roving-items="[role=option]" aria-label="好友">
            {found.slice(0, LISTED).map(p => <button type="button" key={p.username} role="option" aria-selected={p.username === selected?.username}
              className={'conversation-row' + (p.username === selected?.username ? ' active' : '')} onClick={() => { onSelect(p); close(true); }}>
              <Avatar name={p.displayName} url={p.avatarUrl} />
              <span className="conversation-copy"><span className="conversation-name clamp-1">{p.displayName}</span></span>
            </button>)}
            {!found.length && <p className="card-note">没有找到。</p>}
          </div>}
      {found.length > LISTED && <p className="card-note">还有 {n(found.length - LISTED)} 位，搜索名字找到 TA。</p>}
    </Popover>
  </>;
}

const when = (ms: number) => new Date(ms).toLocaleString('zh-CN', { hour12: false, year: 'numeric', month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });

/** A few messages as they went back and forth, mine on the right. */
function Exchange({ label, messages, me, friend }: { label: string; messages: DualMessage[]; me: string; friend: string }) {
  return <ol className="report-exchange" aria-label={label}>
    {messages.map((m, i) => <li key={i} className={m.isSentByMe ? 'mine' : undefined}>
      <span className="report-label">{m.isSentByMe ? me : friend} · {when(m.createTime)}</span>
      <span className="clamp-3">{messageText(m)}</span>
    </li>)}
  </ol>;
}

export function DualReportView({ report }: { report: DualReport }) {
  const r = report;
  const span = r.year ? r.year + ' 年' : '这些年';
  const me = r.selfName || '我';
  const firsts = r.firstChatMessages?.length ? r.firstChatMessages : r.firstChat ? [r.firstChat] : [];
  const yearFirsts = r.year && r.yearFirstChat ? (r.yearFirstChat.firstThreeMessages.length ? r.yearFirstChat.firstThreeMessages : [r.yearFirstChat]) : [];
  const months = r.monthly ? monthBars({ hourlyDistribution: {}, weekdayDistribution: {}, monthlyDistribution: r.monthly }) : [];
  const opened = r.initiative ? r.initiative.initiated + r.initiative.received : 0;
  return <article className="analytics-page report" aria-label={me + '和' + r.friendName + '的' + (r.year ? ' ' + r.year + ' 年' : '') + '双人报告'}>
    <section className="report-hero" aria-label="总览">
      <span className="report-pair"><Avatar name={me} url={r.selfAvatarUrl} /><Avatar name={r.friendName} url={r.friendAvatarUrl} /></span>
      <span className="report-label">{span}，你和{r.friendName}聊了</span>
      <strong className="report-big num">{n(r.stats.totalMessages)}</strong>
      <span className="report-note">条消息，写下 {n(r.stats.totalWords)} 个字，还有 {n(r.stats.imageCount)} 张图片、{n(r.stats.voiceCount)} 段语音和 {n(r.stats.emojiCount)} 个表情。</span>
    </section>

    {firsts.length > 0 && <section className="analytics-block" aria-label="第一次聊天">
      <h2>第一次聊天 <small className="muted-num num">{when(firsts[0].createTime)}</small></h2>
      <Exchange label="最早的几条消息" messages={firsts} me={me} friend={r.friendName} />
    </section>}

    {yearFirsts.length > 0 && <section className="analytics-block" aria-label="今年的第一句">
      <h2>{r.year} 年的第一句</h2>
      <Exchange label={r.year + ' 年最早的几条消息'} messages={yearFirsts} me={me} friend={r.friendName} />
    </section>}

    <section className="analytics-block" aria-label="值得记住的">
      <h2>值得记住的</h2>
      <ul className="report-moments">
        {r.streak && r.streak.days > 0 && <Moment label="连续聊天最久" value={n(r.streak.days) + ' 天'}>{r.streak.startDate} 到 {r.streak.endDate}，每天都有消息。</Moment>}
        {r.initiative && opened > 0 && <Moment label="由你开场" value={Math.round((r.initiative.initiated / opened) * 100) + '%'}>
          {n(r.initiative.initiated)} 次对话由你开始，{n(r.initiative.received)} 次由 TA 开始。</Moment>}
        {r.response && r.response.count > 0 && <Moment label="平均回复" value={formatWait(r.response.avg)}>
          最快 {formatWait(r.response.fastest)}{r.response.slowest !== undefined && <>，最慢 {formatWait(r.response.slowest)}</>}，共 {n(r.response.count)} 次回复。</Moment>}
      </ul>
    </section>

    {r.heatmap && <section className="analytics-block" aria-label="什么时候聊天">
      <h2>什么时候聊天</h2>
      <Heatmap data={r.heatmap} />
    </section>}

    {months.length > 1 && <section className="analytics-block" aria-label="每个月">
      <h2>每个月</h2>
      <Columns label="每个月的消息" bars={months} ticks={(i, b) => (r.year ? (i % 3 === 0 ? b.label.slice(7) : null) : yearStarts(months, i) ? b.label.slice(0, 4) : null)} />
    </section>}

    {(r.topPhrases.length > 0 || r.myExclusivePhrases.length > 0 || r.friendExclusivePhrases.length > 0) && <section className="analytics-block" aria-label="常说的话">
      <h2>常说的话</h2>
      {r.topPhrases.length > 0 && <Phrases phrases={r.topPhrases} label="你们都常说" />}
      <div className="report-likes">
        {r.myExclusivePhrases.length > 0 && <div><span className="report-label">只有你常说</span><Phrases phrases={r.myExclusivePhrases} label="只有你常说" /></div>}
        {r.friendExclusivePhrases.length > 0 && <div><span className="report-label">只有 TA 常说</span><Phrases phrases={r.friendExclusivePhrases} label="只有 TA 常说" /></div>}
      </div>
    </section>}
  </article>;
}

/** The friend picked, and the dual report of the year chosen with them. */
export function DualReportPage({ friend, onFriend, year }: { friend: Friend | null; onFriend(friend: Friend): void; year: number }) {
  const [report, retry] = useRead('dual:' + (friend?.username ?? '') + ':' + year, signal => (friend ? getDualReport(friend.username, year, signal) : Promise.resolve(null)));
  return <div className="group-analytics">
    <FriendPicker selected={friend} onSelect={onFriend} />
    {!friend ? <p className="muted">选一位好友，生成你们两个人的报告。</p>
      : report.status === 'error' ? <Failed title="生成报告失败" message={report.message} onRetry={retry} />
        : report.status === 'loading' || !report.value
          ? <div className="analytics-page" aria-busy="true"><GridSkeleton label={'正在生成和' + friend.displayName + '的报告'} cells={3} /><ListSkeleton label="正在统计" rows={3} /></div>
          : <DualReportView report={report.value} />}
  </div>;
}
