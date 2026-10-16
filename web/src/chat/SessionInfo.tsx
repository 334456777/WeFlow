// The console's 「会话资料」 for real conversations (#200): who it is, how many messages, the first and the latest;
// in a group its members, searchable, each with what they said in it.
import { useEffect, useMemo, useState } from 'react';
import { ArrowLeft, ArrowClockwise, CircleNotch, Crown, MagnifyingGlass, UserCheck } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { ApiError } from '../api/client';
import { getDetail, getDetailExtra, getMemberStats, getPanel, type MemberStats, type PanelMember, type SessionDetail, type SessionDetailExtra } from '../api/details';
import { useLive } from '../api/live';
import { Avatar } from '../Avatar';
import { VirtualList } from '../VirtualList';
import { kindLabels, kindOf } from './sessions';

type Loaded<T> = { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; message: string };

const errorOf = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

const when = (seconds: number | null | undefined) =>
  seconds ? new Date(seconds * 1000).toLocaleString('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }) : '—';

/** Reads `load` for `key`, again on `refresh`; the previous value stays shown while a refresh reads. */
function useLoaded<T>(key: string | null, load: (signal: AbortSignal) => Promise<T>, fallback: string): [Loaded<T>, () => void] {
  const [state, setState] = useState<{ key: string | null; loaded: Loaded<T> }>({ key, loaded: { status: 'loading' } });
  const [round, setRound] = useState(0);
  useEffect(() => {
    if (!key) return;
    const controller = new AbortController();
    setState(old => (old.key === key && old.loaded.status === 'ready' ? old : { key, loaded: { status: 'loading' } }));
    load(controller.signal).then(
      value => { if (!controller.signal.aborted) setState({ key, loaded: { status: 'ready', value } }); },
      problem => { if (!controller.signal.aborted) setState(old => (old.key === key && old.loaded.status === 'ready' ? old : { key, loaded: { status: 'error', message: errorOf(problem, fallback) } })); },
    );
    return () => controller.abort();
    // `load` is a new function each render; `key` and `round` say when to read
  }, [key, round]);
  return [state.key === key ? state.loaded : { status: 'loading' }, () => setRound(n => n + 1)];
}

/** Reads only while `active` (its card open): the first and latest times and the members are a command each. */
export function SessionInfo({ session, active = true }: { session: ChatSession; active?: boolean }) {
  const id = session.username;
  const kind = kindOf(id);
  const key = active ? id : null;
  const [detail, refreshDetail] = useLoaded<SessionDetail>(key, signal => getDetail(id, signal), '读取会话资料失败');
  const [extra, refreshExtra] = useLoaded<SessionDetailExtra>(key, signal => getDetailExtra(id, signal), '读取首条和最新消息的时间失败');
  // a new message changes the count and the latest time (#201)
  useLive(event => {
    if (event.type === 'resumed' || event.sessionId === id) { refreshDetail(); refreshExtra(); }
  });

  const fact = detail.status === 'ready' ? detail.value : null;
  const name = fact?.remark || fact?.displayName || session.displayName || id;
  const pending = <CircleNotch size={12} className="spin" aria-label="正在读取" />;
  return <>
    <div className="info-content"><Avatar name={name} url={fact?.avatarUrl || session.avatarUrl} /><div>
      <h4>{name}</h4>
      <p className="clamp-2">{kindLabels[kind]}{fact?.alias ? ' · 微信号 ' + fact.alias : ''}{fact?.remark && fact.displayName && fact.displayName !== fact.remark ? ' · ' + fact.displayName : ''}</p>
    </div></div>
    {detail.status === 'error' && <p className="card-note" role="alert">{detail.message} <button type="button" className="text-button" onClick={refreshDetail}>重试</button></p>}
    {extra.status === 'error' && <p className="card-note" role="alert">{extra.message} <button type="button" className="text-button" onClick={refreshExtra}>重试</button></p>}
    <dl className="info-grid">
      <div><dt>消息总数</dt><dd className="num">{fact ? fact.messageCount.toLocaleString('zh-CN') + ' 条' : detail.status === 'loading' ? pending : '—'}</dd></div>
      <div><dt>首条消息</dt><dd className="num">{extra.status === 'ready' ? when(extra.value.firstMessageTime) : extra.status === 'loading' ? pending : '—'}</dd></div>
      <div><dt>最新消息</dt><dd className="num">{extra.status === 'ready' ? when(extra.value.latestMessageTime) : extra.status === 'loading' ? pending : when(session.lastTimestamp)}</dd></div>
      <div><dt>ID</dt><dd className="clamp-1 info-id" title={id}>{id}</dd></div>
    </dl>
    {kind === 'group' && <Members chatroomId={id} active={active} />}
  </>;
}

const MEMBER_ROW = 52;

const memberName = (m: PanelMember) => m.groupNickname || m.remark || m.displayName || m.nickname || m.username;

function Members({ chatroomId, active }: { chatroomId: string; active: boolean }) {
  const [members, reload] = useLoaded<PanelMember[]>(active ? chatroomId : null, signal => getPanel(chatroomId, signal), '读取群成员失败');
  const [query, setQuery] = useState('');
  const [chosen, setChosen] = useState<PanelMember | null>(null);
  useEffect(() => { setQuery(''); setChosen(null); }, [chatroomId]);

  const all = members.status === 'ready' ? members.value : [];
  const shown = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return all;
    return all.filter(m => [m.groupNickname, m.remark, m.displayName, m.nickname, m.alias, m.username]
      .some(text => text?.toLocaleLowerCase().includes(needle)));
  }, [all, query]);

  if (chosen) return <MemberDetail chatroomId={chatroomId} member={chosen} active={active} onBack={() => setChosen(null)} />;
  return <section className="members" aria-label="群成员">
    <div className="members-head"><h5>群成员{members.status === 'ready' && <span className="num"> {all.length}</span>}</h5></div>
    {members.status === 'loading' && <p className="empty-text" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在读取群成员</p>}
    {members.status === 'error' && <p className="empty-text" role="alert">{members.message} <button type="button" className="text-button" onClick={reload}><ArrowClockwise size={12} />重试</button></p>}
    {members.status === 'ready' && <>
      <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" /><input type="search" aria-label="搜索群成员" placeholder="搜索群昵称、备注或微信号" value={query} onChange={event => setQuery(event.target.value)} /></div>
      {shown.length
        ? <VirtualList className="member-list scroll-area" aria-label="群成员列表" items={shown} rowHeight={MEMBER_ROW} track=".member-row"
          keyOf={m => m.username} renderRow={m => <button type="button" className="member-row" aria-label={memberName(m) + (m.isOwner ? '，群主' : '') + (m.isFriend ? '，好友' : '')} onClick={() => setChosen(m)}>
            <Avatar name={memberName(m)} url={m.avatarUrl} />
            <span className="member-copy"><span className="clamp-1">{memberName(m)}</span>{memberName(m) !== (m.remark || m.displayName) && (m.remark || m.displayName) && <small className="clamp-1">{m.remark || m.displayName}</small>}</span>
            {m.isOwner && <span className="badge member-badge"><Crown size={12} aria-hidden="true" />群主</span>}
            {m.isFriend && <span className="badge member-badge quiet"><UserCheck size={12} aria-hidden="true" />好友</span>}
          </button>} />
        : <p className="empty-text">没有匹配的成员。</p>}
    </>}
  </section>;
}

function MemberDetail({ chatroomId, member, active, onBack }: { chatroomId: string; member: PanelMember; active: boolean; onBack: () => void }) {
  const [stats, reload] = useLoaded<MemberStats>(active ? chatroomId + '\n' + member.username : null, signal => getMemberStats(chatroomId, member.username, signal), '读取成员统计失败');
  const name = memberName(member);
  const s = stats.status === 'ready' ? stats.value.statistics : null;
  // the hour of day they write most in
  const busiest = stats.status === 'ready'
    ? Object.entries(stats.value.timeDistribution).reduce<[string, number] | null>((best, entry) => (!best || entry[1] > best[1] ? entry : best), null)
    : null;
  return <section className="members" aria-label={name + '的群内统计'}>
    <div className="members-head"><button type="button" className="icon-button small" aria-label="返回群成员" onClick={onBack}><ArrowLeft size={16} /></button><h5 className="clamp-1">{name}</h5></div>
    <div className="info-content"><Avatar name={name} url={member.avatarUrl} /><div><p className="clamp-2">{[member.isOwner && '群主', member.isFriend && '好友', member.displayName !== name && member.displayName, member.username].filter(Boolean).join(' · ')}</p></div></div>
    {stats.status === 'loading' && <p className="empty-text" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在统计</p>}
    {stats.status === 'error' && <p className="empty-text" role="alert">{stats.message} <button type="button" className="text-button" onClick={reload}>重试</button></p>}
    {s && <>
      <dl className="info-grid">
        <div><dt>群内消息</dt><dd className="num">{s.totalMessages.toLocaleString('zh-CN')} 条</dd></div>
        <div><dt>活跃天数</dt><dd className="num">{s.activeDays} 天</dd></div>
        <div><dt>文字 / 图片</dt><dd className="num">{s.textMessages} / {s.imageMessages}</dd></div>
        <div><dt>语音 / 视频 / 表情</dt><dd className="num">{s.voiceMessages} / {s.videoMessages} / {s.emojiMessages}</dd></div>
        <div><dt>首条</dt><dd className="num">{when(s.firstMessageTime)}</dd></div>
        <div><dt>最近</dt><dd className="num">{when(s.lastMessageTime)}</dd></div>
        {busiest && busiest[1] > 0 && <div><dt>最常发言</dt><dd className="num">{busiest[0]}:00 – {Number(busiest[0]) + 1}:00</dd></div>}
      </dl>
      {stats.status === 'ready' && stats.value.commonPhrases.length > 0 && <div className="member-phrases"><h6>常说</h6><ul>{stats.value.commonPhrases.slice(0, 6).map(p => <li key={p.phrase}><span className="clamp-1">{p.phrase}</span><small className="num">{p.count}</small></li>)}</ul></div>}
    </>}
  </section>;
}
