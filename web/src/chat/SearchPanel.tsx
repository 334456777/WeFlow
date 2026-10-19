// The console's search card for real conversations (#199): search this conversation or all of them (as WeChat
// matches), within a time range; a hit opens its conversation there. A calendar with the days that have messages
// jumps to a day.
import { useEffect, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { ArrowSquareOut, CalendarBlank, CircleNotch, MagnifyingGlass } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import type { SearchHit } from '../api/messages';
import { DateField } from '../calendar/DateField';
import { DateRange, type Range } from '../calendar/DateRange';
import { addDays, parseDay } from '../calendar/dates';
import { ResultsSkeleton } from '../Skeleton';
import { HoverTrack } from '../HoverTrack';
import { Segmented } from '../Segmented';
import { loadMembers, memberName, membersKnown } from './LiveMessages';
import { dayOf, formatClock, isMine } from './messages';
import { kindOf } from './sessions';
import { useDateCounts } from './useDateCounts';
import { useSearch, type SearchState } from './useSearch';

type Scope = 'session' | 'all';

/**
 * The start of a day and the end of another, in Unix seconds; 0 where open. The end is the second before the next
 * day's local midnight, as a day is not always 24 hours long (23 or 25 where the clocks change).
 */
export function secondsOf(range: Range): { start: number; end: number } {
  return {
    start: range.from ? parseDay(range.from).getTime() / 1000 : 0,
    end: range.to ? parseDay(addDays(range.to, 1)).getTime() / 1000 - 1 : 0,
  };
}

/** The text with each match of the keyword marked. */
function Marked({ text, keyword }: { text: string; keyword: string }) {
  const lower = text.toLocaleLowerCase();
  const needle = keyword.toLocaleLowerCase();
  if (!needle) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  for (let found = lower.indexOf(needle); found >= 0; found = lower.indexOf(needle, at)) {
    if (found > at) parts.push(text.slice(at, found));
    parts.push(<mark key={found}>{text.slice(found, found + needle.length)}</mark>);
    at = found + needle.length;
  }
  parts.push(text.slice(at));
  return <>{parts}</>;
}

/** The card's summary: how many hits, or what a search would cover. */
export function searchSummary(state: SearchState): string {
  if (state.status === 'ready') return state.hits.length + (state.hasMore ? '+' : '') + ' 条结果';
  if (state.status === 'loading') return '正在搜索';
  return state.query && !state.query.sessionId ? '全部会话' : '当前会话';
}

export function SearchPanel({ session, sessions, search, onOpenHit, onJumpDay }: {
  session: ChatSession | undefined;
  sessions: ChatSession[];
  search: ReturnType<typeof useSearch>;
  onOpenHit: (hit: SearchHit) => void;
  onJumpDay: (sessionId: string, day: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState<Scope>('session');
  const [datesOpen, setDatesOpen] = useState(false);
  const [range, setRange] = useState<Range>({ from: '', to: '' });
  const [jumpDay, setJumpDay] = useState('');
  const sessionId = session?.username ?? null;
  const { counts, request } = useDateCounts(sessionId);
  const { state } = search;
  const name = session?.displayName || sessionId || '';
  // the names of the senders in the groups among the hits, one group after another
  const [, setNamed] = useState(0);
  useEffect(() => {
    const groups = [...new Set(state.hits.map(h => h._session_id))].filter(id => kindOf(id) === 'group' && !membersKnown(id));
    if (!groups.length) return;
    const controller = new AbortController();
    (async () => {
      for (const id of groups) {
        try { await loadMembers(id, controller.signal); } catch { /* the id stays */ }
        if (controller.signal.aborted) return;
        setNamed(n => n + 1);
      }
    })();
    return () => controller.abort();
  }, [state.hits]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const keyword = query.trim();
    if (!keyword) { search.clear(); return; }
    if (scope === 'session' && !sessionId) return;
    search.search({ keyword, sessionId: scope === 'session' ? sessionId : null, ...(datesOpen ? secondsOf(range) : { start: 0, end: 0 }) });
  };

  const nameOfSession = (id: string) => sessions.find(s => s.username === id)?.displayName || id;
  const senderOf = (hit: SearchHit) => {
    if (isMine(hit)) return '我';
    const sender = hit.senderUsername;
    if (kindOf(hit._session_id) === 'group') return (sender && memberName(hit._session_id, sender)) || sender || '';
    return nameOfSession(hit._session_id);
  };
  const titleOf = (hit: SearchHit) => {
    if (state.query?.sessionId) return senderOf(hit);
    const where = nameOfSession(hit._session_id);
    return kindOf(hit._session_id) === 'group' ? where + ' · ' + senderOf(hit) : where;
  };
  const keyword = state.query?.keyword ?? '';

  return <>
    <form className="search-form" onSubmit={submit}>
      <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label={scope === 'all' ? '搜索全部会话的消息' : '搜索此会话的消息'} placeholder={scope === 'all' ? '搜索全部会话的消息' : '搜索' + name + '的消息'} value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      <button className="primary-button search-button" type="submit" aria-label="搜索"><MagnifyingGlass size={16} /><span className="search-button-label">搜索</span></button>
      <button className={'icon-button' + (datesOpen ? ' on' : '')} type="button" aria-label="筛选搜索日期" aria-expanded={datesOpen} onClick={() => setDatesOpen(open => !open)}><CalendarBlank size={16} /></button>
    </form>
    <div className="search-date-row"><span className="field-label">范围</span>
      <Segmented className="scope-group" count={2} index={scope === 'all' ? 1 : 0} role="group" aria-label="搜索范围">
        <button type="button" aria-pressed={scope === 'session'} onClick={() => setScope('session')}>当前会话</button>
        <button type="button" aria-pressed={scope === 'all'} onClick={() => setScope('all')}>全部会话</button>
      </Segmented>
    </div>
    {datesOpen && <div className="search-date-row"><span className="field-label">时间范围</span>
      <DateRange label="搜索的时间范围" from={range.from} to={range.to} onChange={setRange}
        marks={scope === 'session' ? counts : null} onOpen={scope === 'session' ? request : undefined} /></div>}
    <div className="search-date-row"><span className="field-label">跳到日期</span>
      <DateField label="跳到日期" placeholder="选择一天" value={jumpDay} marks={counts} onOpen={request}
        onChange={day => { setJumpDay(day); if (sessionId) onJumpDay(sessionId, day); }} />
    </div>
    <div className="search-results" aria-live="polite" aria-busy={state.status === 'loading'} data-roving="vertical" data-roving-items=".search-result"><HoverTrack items=".search-result" />{
      state.status === 'idle' ? <p className="empty-text">输入关键词，搜索{scope === 'all' ? '全部会话' : '当前会话'}的消息。</p>
        : state.status === 'loading' ? <ResultsSkeleton label="正在搜索" />
          : state.status === 'error' ? <p className="empty-text">{state.error}</p>
            : state.hits.length
              ? <><p className="results-count">{state.hasMore ? '已显示 ' : '找到 '}{state.hits.length} 条记录<span>点击可定位到消息</span></p>
                {state.hits.map(hit => <button className="search-result" type="button" key={hit._session_id + ' ' + hit._cursor} onClick={() => onOpenHit(hit)}>
                  <span className={'search-result-copy' + (state.query?.sessionId ? '' : ' stacked')}><strong className="clamp-1">{titleOf(hit)}</strong><span className="clamp-1"><Marked text={hit.parsedContent} keyword={keyword} /></span></span>
                  <time>{dayOf(hit.createTime)} {formatClock(hit.createTime)}</time><ArrowSquareOut size={14} />
                </button>)}
                {state.error && <p className="empty-text">{state.error}</p>}
                {state.hasMore && <button type="button" className="secondary-button more-button" disabled={state.loadingMore} onClick={search.more}>{state.loadingMore ? <><CircleNotch size={14} className="spin" />正在加载</> : '加载更多'}</button>}</>
              : <p className="empty-text">没有找到匹配的消息，试试其他关键词或时间范围。</p>
    }</div>
  </>;
}
