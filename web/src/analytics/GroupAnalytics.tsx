// Group chat analytics (#207): a group picked from the list, its messages by kind and hour and who sends the most,
// one member's numbers, words and messages, and the console cards that export them as jobs of the server.
import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowClockwise, ArrowLeft, CaretDown, CircleNotch, DownloadSimple, MagnifyingGlass, UsersThree } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { ApiError } from '../api/client';
import { hourBars, typeBars } from '../api/analytics';
import {
  getGroupHours, getGroupMedia, getGroupRanking, getMember, getMemberMessages, memberMessagesExport, membersExport,
  type DayRange, type Group, type GroupRanked, type MemberMessage,
} from '../api/groups';
import { runJob } from '../jobs/useJobs';
import { useRead } from '../useRead';
import { Popover } from '../Popover';
import { GridSkeleton, ListSkeleton, ResultsSkeleton } from '../Skeleton';
import { Columns, day, n, Shares } from './AnalyticsViews';
import type { GroupsState } from './useAnalytics';

const Failed = ({ message, onRetry }: { message: string; onRetry(): void }) =>
  <div className="pane-message" role="alert"><strong>读取失败</strong><p>{message}</p>
    <button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={12} />重试</button></div>;

/** Up to this many groups are listed; the rest are found by name. */
const LISTED = 50;

/** The group shown, and the list to pick another from, found by name. */
export function GroupPicker({ state, selected, onSelect, onRetry }: { state: GroupsState; selected: Group | null; onSelect(group: string): void; onRetry(): void }) {
  const [open, setOpen] = useState(false);
  const [keyword, setKeyword] = useState('');
  const anchor = useRef<HTMLButtonElement>(null);
  const groups = state.status === 'ready' ? state.groups : [];
  const found = useMemo(() => {
    const words = keyword.trim().toLowerCase();
    return words ? groups.filter(g => g.displayName.toLowerCase().includes(words)) : groups;
  }, [groups, keyword]);
  if (state.status === 'error') return <Failed message={state.message} onRetry={onRetry} />;
  if (state.status !== 'ready') return <ListSkeleton label="正在读取群聊" rows={1} />;
  if (!groups.length) return <p className="muted">没有群聊。</p>;
  const close = (focus: boolean) => { if (focus) anchor.current?.focus(); setOpen(false); };
  return <>
    <button ref={anchor} type="button" className="group-picker" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
      <span className="avatar avatar-initial" aria-hidden="true"><UsersThree size={20} /></span>
      <span className="rank-copy"><strong className="clamp-1">{selected?.displayName ?? '选择群聊'}</strong>
        <span className="cache-text">{selected ? n(selected.memberCount) + ' 位成员 · 点这里换一个群' : '共 ' + n(groups.length) + ' 个群聊'}</span></span>
      <CaretDown size={14} aria-hidden="true" />
    </button>
    <Popover anchor={anchor} open={open} onClose={reason => close(reason === 'escape')} label="选择群聊" className="group-popover">
      <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
        <input type="search" autoFocus aria-label="搜索群聊" placeholder="搜索群名" value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
      <div className="group-options" role="listbox" data-roving="vertical" data-roving-items="[role=option]" aria-label="群聊">
        {found.slice(0, LISTED).map(g => <button type="button" key={g.username} role="option" aria-selected={g.username === selected?.username}
          className={'conversation-row' + (g.username === selected?.username ? ' active' : '')} onClick={() => { onSelect(g.username); close(true); }}>
          <span className="conversation-copy"><span className="conversation-name clamp-1">{g.displayName}</span>
            <span className="conversation-preview">{n(g.memberCount)} 位成员</span></span>
        </button>)}
        {!found.length && <p className="card-note">没有找到。</p>}
      </div>
      {found.length > LISTED && <p className="card-note">还有 {n(found.length - LISTED)} 个，搜索群名找到它们。</p>}
    </Popover>
  </>;
}

function MemberRow({ rank, ranked, top, onOpen }: { rank: number; ranked: GroupRanked; top: number; onOpen(): void }) {
  const { member, messageCount } = ranked;
  return <li className="rank-row">
    <span className="rank-number num" aria-hidden="true">{rank}</span>
    <Avatar name={member.displayName} url={member.avatarUrl} />
    <span className="rank-copy">
      <span className="rank-line"><strong>{member.displayName}</strong><span className="num">{n(messageCount)} 条</span></span>
      <span className="rank-split" aria-hidden="true"><span style={{ width: (top ? (messageCount / top) * 100 : 0) + '%' }} /></span>
    </span>
    <button type="button" className="secondary-button" aria-label={'查看' + member.displayName + '的统计'} onClick={onOpen}>查看</button>
  </li>;
}

/** One group: its messages by kind and by hour, and the twenty members who send the most. */
export function GroupStats({ group, range, onMember }: { group: Group; range: DayRange; onMember(member: { username: string; displayName: string; avatarUrl?: string }): void }) {
  const [state, retry] = useRead(group.username + '|' + range.from + '|' + range.to, signal =>
    Promise.all([getGroupMedia(group.username, range, signal), getGroupHours(group.username, range, signal), getGroupRanking(group.username, range, signal)]));
  if (state.status === 'error') return <Failed message={state.message} onRetry={retry} />;
  if (state.status === 'loading') return <div className="analytics-page" aria-busy="true"><GridSkeleton label={'正在统计「' + group.displayName + '」'} cells={4} /><ListSkeleton label="正在读取活跃成员" rows={5} /></div>;
  const [media, hours, ranking] = state.value;
  const top = ranking[0]?.messageCount ?? 0;
  return <div className="analytics-page">
    <section className="analytics-block" aria-label="消息类型">
      <h2>消息类型 <small className="muted-num num">共 {n(media.total)} 条</small></h2>
      {media.total ? <Shares label="各类消息的条数" bars={media.typeCounts.map(t => ({ label: t.name, value: t.count }))} total={media.total} /> : <p className="muted">这段时间没有消息。</p>}
    </section>
    <section className="analytics-block" aria-label="一天中的时段">
      <h2>一天中的时段</h2>
      <Columns label="每小时的消息" bars={hourBars(hours.hourlyDistribution)} ticks={i => (i % 6 === 0 ? String(i) : null)} />
    </section>
    <section className="analytics-block" aria-label="最活跃的成员">
      <h2>最活跃的成员</h2>
      {ranking.length
        ? <ol className="rank-list">{ranking.map((r, i) => <MemberRow key={r.member.username} rank={i + 1} ranked={r} top={top} onOpen={() => onMember(r.member)} />)}</ol>
        : <p className="muted">这段时间没有人发言。</p>}
    </section>
  </div>;
}

const clock = (seconds: number) => new Date(seconds * 1000).toLocaleString('zh-CN', { hour12: false });

/** One member in one group: their numbers, hours, words and emoji, and their messages, newest first. */
export function MemberView({ group, member, range, onBack }: { group: Group; member: { username: string; displayName: string; avatarUrl?: string }; range: DayRange; onBack(): void }) {
  const key = group.username + '|' + member.username + '|' + range.from + '|' + range.to;
  const [state, retry] = useRead(key, signal => getMember(group.username, member.username, range, signal));
  const [page, setPage] = useState<{ messages: MemberMessage[]; more: boolean; cursor: number; loading: boolean; error?: string }>({ messages: [], more: false, cursor: 0, loading: true });
  const readPage = (cursor: number, signal?: AbortSignal) => {
    setPage(p => ({ ...p, loading: true, error: undefined }));
    getMemberMessages(group.username, member.username, range, cursor, signal).then(next => setPage(p => ({
      messages: cursor ? [...p.messages, ...next.messages] : next.messages, more: next.hasMore, cursor: next.nextCursor, loading: false,
    })), problem => { if (!signal?.aborted) setPage(p => ({ ...p, loading: false, error: (problem instanceof ApiError && problem.message) || '读取消息失败' })); });
  };
  useEffect(() => {
    const controller = new AbortController();
    setPage({ messages: [], more: false, cursor: 0, loading: true });
    readPage(0, controller.signal);
    return () => controller.abort();
  }, [key]);

  const head = <div className="member-head">
    <button type="button" className="icon-button" aria-label={'返回「' + group.displayName + '」的统计'} onClick={onBack}><ArrowLeft size={18} /></button>
    <Avatar name={member.displayName} url={member.avatarUrl} />
    <span className="rank-copy"><strong className="clamp-1">{member.displayName}</strong><span className="cache-text clamp-1">在「{group.displayName}」里</span></span>
  </div>;
  if (state.status === 'error') return <>{head}<Failed message={state.message} onRetry={retry} /></>;
  if (state.status === 'loading') return <div className="analytics-page" aria-busy="true">{head}<GridSkeleton label={'正在统计' + member.displayName + '的消息'} cells={4} /></div>;
  const { statistics: s, timeDistribution, commonPhrases, commonEmojis } = state.value;
  return <div className="analytics-page">
    {head}
    <section className="analytics-block" aria-label="成员总览">
      <dl className="info-grid analytics-numbers">
        <div><dt>消息</dt><dd className="num">{n(s.totalMessages)} 条</dd></div>
        <div><dt>发过言的天数</dt><dd className="num">{n(s.activeDays)} 天</dd></div>
        <div><dt>第一条</dt><dd className="num">{day(s.firstMessageTime)}</dd></div>
        <div><dt>最近一条</dt><dd className="num">{day(s.lastMessageTime)}</dd></div>
      </dl>
      {s.totalMessages > 0 && <Shares label="各类消息的条数" bars={typeBars(s)} total={s.totalMessages} />}
    </section>
    <section className="analytics-block" aria-label="成员的时段">
      <h2>一天中的时段</h2>
      <Columns label="每小时的消息" bars={hourBars(timeDistribution)} ticks={i => (i % 6 === 0 ? String(i) : null)} />
    </section>
    {(commonPhrases.length > 0 || commonEmojis.length > 0) && <section className="analytics-block" aria-label="常说的话">
      <h2>常说的话</h2>
      <ul className="word-chips" aria-label="常用语">{commonPhrases.map(p => <li key={p.phrase}><span className="clamp-1">{p.phrase}</span><small className="num">{n(p.count)}</small></li>)}</ul>
      {commonEmojis.length > 0 && <ul className="word-chips" aria-label="常用表情">{commonEmojis.map(e => <li key={e.emoji}>{e.emoji}<small className="num">{n(e.count)}</small></li>)}</ul>}
    </section>}
    <section className="analytics-block" aria-label="成员的消息">
      <h2>消息</h2>
      {page.messages.length > 0 && <ol className="member-messages">{page.messages.map(m => <li key={m.messageKey}>
        <time className="cache-text num" dateTime={new Date(m.createTime * 1000).toISOString()}>{clock(m.createTime)}</time>
        <span className="clamp-3">{m.parsedContent || '[消息]'}</span></li>)}</ol>}
      {page.loading && !page.messages.length && <ResultsSkeleton label="正在读取消息" rows={3} />}
      {!page.loading && !page.error && !page.messages.length && <p className="muted">这段时间没有消息。</p>}
      {page.error && <p className="card-note setting-warning" role="alert">{page.error}</p>}
      {(page.more || page.error) && page.messages.length > 0 && <button type="button" className="secondary-button" disabled={page.loading} onClick={() => readPage(page.cursor)}>
        {page.loading ? <><CircleNotch size={12} className="spin" />正在读取</> : '显示更早的消息'}</button>}
    </section>
  </div>;
}

/** The console card: the group's member list, and the messages of the member shown, as files kept by the server. */
export function GroupExport({ group, member, range }: { group: Group | null; member: { username: string; displayName: string } | null; range: DayRange }) {
  const [busy, setBusy] = useState<'members' | 'messages' | null>(null);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  useEffect(() => { setNote(null); }, [group?.username, member?.username]);
  if (!group) return <p className="card-note">先选一个群聊。</p>;
  const start = async (what: 'members' | 'messages') => {
    setBusy(what); setNote(null);
    try {
      if (what === 'members') await runJob('导出群成员 · ' + group.displayName, 'POST', 'group/export-members', membersExport(group.username));
      else if (member) await runJob('导出' + member.displayName + '的群消息 · ' + group.displayName, 'POST', 'group/export-member-messages', memberMessagesExport(group.username, member.username, range));
      setNote({ text: '已加入任务中心，完成后在那里下载 Excel 文件。' });
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || '导出没有开始，请重试。', error: true });
    } finally {
      setBusy(null);
    }
  };
  return <div className="excluded-people">
    <button type="button" className="secondary-button" disabled={busy !== null} onClick={() => start('members')}>
      {busy === 'members' ? <CircleNotch size={12} className="spin" /> : <DownloadSimple size={12} />}导出成员列表</button>
    {member && <button type="button" className="secondary-button" disabled={busy !== null} onClick={() => start('messages')}>
      {busy === 'messages' ? <CircleNotch size={12} className="spin" /> : <DownloadSimple size={12} />}导出{member.displayName}的消息</button>}
    <p className="card-note">{member ? '消息按「时间范围」导出。' : '在「最活跃的成员」里点「查看」，可以导出这个成员的消息。'}</p>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </div>;
}
