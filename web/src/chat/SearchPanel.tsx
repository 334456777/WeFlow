// 操作控制台的搜索卡片（#199）：在当前会话或全部会话中按时间范围搜索，匹配的是微信自己的搜索所匹配的文字；
// 点结果就在那个位置打开它的会话。标出有消息的日子的日历可以跳到某一天。
import { useEffect, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { ArrowSquareOut, CircleNotch, MagnifyingGlass } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import type { SearchHit, SearchQuery } from '../api/messages';
import { DateField } from '../calendar/DateField';
import { DateRange, type Range } from '../calendar/DateRange';
import { addDays, parseDay, toDay } from '../calendar/dates';
import { ResultsSkeleton } from '../Skeleton';
import { HoverTrack } from '../HoverTrack';
import { Segmented } from '../Segmented';
import { loadMembers, memberName, membersKnown } from './LiveMessages';
import { dayOf, formatClock } from '../format';
import { isMine } from './messages';
import { kindOf } from '../sessionKind';
import { useDateCounts } from './useDateCounts';
import { useSearch, type SearchState } from './useSearch';
import { t } from '../i18n';

type Scope = 'session' | 'all';

/**
 * 一天的开始和另一天的结束，单位是 Unix 秒，不限的一端为 0。结束是后一天本地零点的前一秒，因为一天不总是
 * 24 小时（实行夏令时的地方调钟那天是 23 或 25 小时）。
 */
export function secondsOf(range: Range): { start: number; end: number } {
  return {
    start: range.from ? parseDay(range.from).getTime() / 1000 : 0,
    end: range.to ? parseDay(addDays(range.to, 1)).getTime() / 1000 - 1 : 0,
  };
}

/** 搜索限定的日期，由 `secondsOf` 换算出的秒数还原；不限的一端和没有搜索时为空。 */
export function rangeOf(query: SearchQuery | null): Range {
  return {
    from: query?.start ? toDay(new Date(query.start * 1000)) : '',
    to: query?.end ? toDay(new Date(query.end * 1000)) : '',
  };
}

/** 搜索的是全部会话还是一个会话；没有搜索时算一个会话。 */
export const scopeOf = (query: SearchQuery | null): Scope => (query && !query.sessionId ? 'all' : 'session');

/** 标出关键词每一处匹配（不区分大小写）的文字。 */
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

/** 卡片的摘要：结果的条数，或者搜索的范围。 */
export function searchSummary(state: SearchState): string {
  if (state.status === 'ready') return t('{n} 条结果', { n: state.hits.length + (state.hasMore ? '+' : '') });
  if (state.status === 'loading') return t('正在搜索');
  return scopeOf(state.query) === 'all' ? t('全部会话') : t('当前会话');
}

/** 目前的结果条数，以及这些结果是在哪个范围搜索的：之后选的范围可能已经不同。 */
function foundIn(state: SearchState): string {
  const all = scopeOf(state.query) === 'all';
  const n = state.hits.length;
  if (state.hasMore) return all ? t('已显示 {n} 条记录（全部会话）', { n }) : t('已显示 {n} 条记录（当前会话）', { n });
  return all ? t('找到 {n} 条记录（全部会话）', { n }) : t('找到 {n} 条记录（当前会话）', { n });
}

/** 搜索卡片：查询条件、结果，以及标出会话里有消息的日子的日历。 */
export function SearchPanel({ session, sessions, search, onOpenHit, onJumpDay }: {
  session: ChatSession | undefined;
  sessions: ChatSession[];
  search: ReturnType<typeof useSearch>;
  onOpenHit: (hit: SearchHit) => void;
  onJumpDay: (sessionId: string, day: string) => void;
}) {
  // 搜索属于页面，卡片卸载后仍在：卡片重新挂载时，从当前结果所用的查询条件开始
  const [query, setQuery] = useState(() => search.state.query?.keyword ?? '');
  const [scope, setScope] = useState<Scope>(() => scopeOf(search.state.query));
  const [range, setRange] = useState<Range>(() => rangeOf(search.state.query));
  const [jumpDay, setJumpDay] = useState('');
  const sessionId = session?.username ?? null;
  const { counts, request } = useDateCounts(sessionId);
  const { state } = search;
  const name = session?.displayName || sessionId || '';
  // 结果中群消息的发送者名字，一个群一个群地读取
  const [, setNamed] = useState(0);
  useEffect(() => {
    const groups = [...new Set(state.hits.map(h => h._session_id))].filter(id => kindOf(id) === 'group' && !membersKnown(id));
    if (!groups.length) return;
    const controller = new AbortController();
    (async () => {
      for (const id of groups) {
        try { await loadMembers(id, controller.signal); } catch { /* 读不到时仍显示用户名 */ }
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
    search.search({ keyword, sessionId: scope === 'session' ? sessionId : null, ...secondsOf(range) });
  };

  const nameOfSession = (id: string) => sessions.find(s => s.username === id)?.displayName || id;
  const senderOf = (hit: SearchHit) => {
    if (isMine(hit)) return t('我');
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
        <input type="search" aria-label={scope === 'all' ? t('搜索全部会话的消息') : t('搜索此会话的消息')} placeholder={scope === 'all' ? t('搜索全部会话的消息') : t('搜索{name}的消息', { name })} value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      <button className="primary-button search-button" type="submit" aria-label={t('搜索')}><MagnifyingGlass size={16} /><span className="search-button-label">{t('搜索')}</span></button>
    </form>
    <div className="search-date-row"><span className="field-label">{t('范围')}</span>
      <Segmented className="scope-group" count={2} index={scope === 'all' ? 1 : 0} role="group" aria-label={t('搜索范围')}>
        <button type="button" aria-pressed={scope === 'session'} onClick={() => setScope('session')}>{t('当前会话')}</button>
        <button type="button" aria-pressed={scope === 'all'} onClick={() => setScope('all')}>{t('全部会话')}</button>
      </Segmented>
    </div>
    {/* 一直显示，可以不填：不选日期就不限时间（#345） */}
    <div className="search-date-row"><span className="field-label">{t('时间范围')}</span>
      <DateRange label={t('搜索的时间范围')} from={range.from} to={range.to} onChange={setRange}
        marks={scope === 'session' ? counts : null} onOpen={scope === 'session' ? request : undefined} /></div>
    <div className="search-date-row"><span className="field-label">{t('跳到日期')}</span>
      <DateField label={t('跳到日期')} placeholder={t('选择一天')} value={jumpDay} marks={counts} onOpen={request}
        onChange={day => { setJumpDay(day); if (sessionId) onJumpDay(sessionId, day); }} />
    </div>
    <div className="search-results" aria-live="polite" aria-busy={state.status === 'loading'} data-roving="vertical" data-roving-items=".search-result"><HoverTrack items=".search-result" />{
      state.status === 'idle' ? <p className="empty-text">{scope === 'all' ? t('输入关键词，搜索全部会话的消息。') : t('输入关键词，搜索当前会话的消息。')}</p>
        : state.status === 'loading' ? <ResultsSkeleton label={t('正在搜索')} />
          : state.status === 'error' ? <p className="empty-text">{state.error}</p>
            : state.hits.length
              ? <><p className="results-count">{foundIn(state)}<span>{t('点击可定位到消息')}</span></p>
                {state.hits.map(hit => <button className="search-result as-row" type="button" key={hit._session_id + ' ' + hit._cursor} onClick={() => onOpenHit(hit)}>
                  <span className={'search-result-copy' + (state.query?.sessionId ? '' : ' stacked')}><strong className="clamp-1">{titleOf(hit)}</strong><span className="clamp-1"><Marked text={hit.parsedContent} keyword={keyword} /></span></span>
                  <time>{dayOf(hit.createTime)} {formatClock(hit.createTime)}</time><ArrowSquareOut size={14} />
                </button>)}
                {state.error && <p className="empty-text">{state.error}</p>}
                {state.hasMore && <button type="button" className="secondary-button more-button" disabled={state.loadingMore} onClick={search.more}>{state.loadingMore ? <><CircleNotch size={14} className="spin" />{t('正在加载')}</> : t('加载更多')}</button>}</>
              : <p className="empty-text">{t('没有找到匹配的消息，试试其他关键词或时间范围。')}</p>
    }</div>
  </>;
}
