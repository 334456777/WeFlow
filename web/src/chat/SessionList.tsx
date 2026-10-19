// The conversation list of the chat page (#197): search, a filter by kind, folded groups, and a virtual list.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { ArrowClockwise, ArrowLeft, BellSlash, MagnifyingGlass, UsersThree } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { Avatar } from '../Avatar';
import { Segmented } from '../Segmented';
import { VirtualList, type VirtualListHandle } from '../VirtualList';
import { filterLabels, FOLDED_ID, formatSessionTime, sessionRows, type SessionFilter, type SessionRow } from './sessions';
import type { SessionsState } from './useSessions';
import { ListSkeleton } from '../Skeleton';

const FILTERS: SessionFilter[] = ['all', 'private', 'group', 'official'];
/** A row is 56px high with 2px to the next (#192). */
const ROW_HEIGHT = 58;
const LIST_PADDING = 8;
const SKELETON_ROWS = 8;

function SessionItem({ row, active, now, onClick }: { row: SessionRow; active: boolean; now: Date; onClick(): void }) {
  return <button type="button" data-row-id={row.id} className={'conversation-row' + (active ? ' active' : '') + (row.muted ? ' muted' : '')}
    aria-label={row.folded ? '打开折叠的群聊，共 ' + row.folded.count + ' 个' : '选择' + row.name} aria-pressed={row.folded ? undefined : active} onClick={onClick}>
    {row.folded ? <span className="avatar avatar-initial" aria-hidden="true"><UsersThree size={20} /></span> : <Avatar name={row.name} url={row.avatarUrl} />}
    <span className="conversation-copy">
      <span className="conversation-head"><span className="conversation-name">{row.name}</span><time className="conversation-time">{formatSessionTime(row.time, now)}</time></span>
      <span className="conversation-foot">
        <span className="conversation-preview clamp-1">{row.preview}</span>
        {row.muted && !row.folded && <BellSlash className="muted-icon" size={14} aria-label="免打扰" />}
        {row.unread > 0 && (row.muted
          ? <span className="unread-dot" aria-label={row.unread + ' 条未读'} />
          : <span className="unread-count" aria-label={row.unread + ' 条未读'}>{row.unread > 99 ? '99+' : row.unread}</span>)}
      </span>
    </span>
  </button>;
}

export function SessionList({ state, sample, selectedId, onSelect, onRetry, officials = [], onFilter }: {
  state: SessionsState;
  /** Sample sessions, shown while no server is connected. */
  sample: boolean;
  selectedId: string | null;
  onSelect(id: string): void;
  onRetry(): void;
  /** Official accounts followed without a conversation, for the 「公众号」 filter (#203). */
  officials?: readonly ChatSession[];
  onFilter?(filter: SessionFilter): void;
}) {
  const [filter, setFilterState] = useState<SessionFilter>('all');
  const setFilter = (next: SessionFilter) => { setFilterState(next); onFilter?.(next); };
  const [query, setQuery] = useState('');
  const [foldedView, setFoldedView] = useState(false);
  const list = useRef<VirtualListHandle>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const now = useMemo(() => new Date(), [state]);

  const rows = useMemo(
    () => (state.status === 'ready' ? sessionRows(state.sessions, state.statuses, filter, query, foldedView, officials) : []),
    [state, filter, query, foldedView, officials],
  );
  const foldedCount = state.status === 'ready' ? state.sessions.filter(s => state.statuses[s.username]?.isFolded).length : 0;
  useEffect(() => { if (foldedView && !foldedCount) setFoldedView(false); }, [foldedView, foldedCount]);

  // Focus a row once it is rendered: the list draws only the rows in sight.
  useEffect(() => {
    if (!focusId) return;
    document.querySelector<HTMLElement>(`.session-pane [data-row-id="${CSS.escape(focusId)}"]`)?.focus();
    setFocusId(null);
  }, [focusId, rows]);

  const open = (row: SessionRow) => {
    if (row.id === FOLDED_ID) { setFoldedView(true); setQuery(''); return; }
    onSelect(row.id);
  };

  // Up and down move to the next conversation and open it, as in WeChat; Home and End go to either end.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const keys: Record<string, (i: number) => number> = {
      ArrowDown: i => i + 1, ArrowUp: i => i - 1, Home: () => 0, End: () => rows.length - 1,
    };
    const move = keys[event.key];
    const current = (event.target as HTMLElement).closest<HTMLElement>('[data-row-id]')?.dataset.rowId;
    if (!move || event.altKey || event.ctrlKey || event.metaKey || !current) return;
    event.preventDefault();
    const index = Math.max(0, Math.min(rows.length - 1, move(rows.findIndex(row => row.id === current))));
    const row = rows[index];
    if (!row) return;
    list.current?.scrollToIndex(index);
    setFocusId(row.id);
    if (row.id !== FOLDED_ID) onSelect(row.id);
  };

  let body;
  if (state.status === 'idle' || state.status === 'loading') {
    body = <ListSkeleton label="正在读取会话" rows={SKELETON_ROWS} />;
  } else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />重试</button></div>;
  } else if (!rows.length) {
    body = <p className="session-message">{query.trim() ? '没有名称匹配的会话。' : state.sessions.length ? '这一类没有会话。' : '这个账号还没有会话。'}</p>;
  } else {
    body = <VirtualList ref={list} data-roving="vertical" data-roving-items=".conversation-row" data-roving-keys="off" className="conversation-list scroll-area" aria-label="会话列表" items={rows} rowHeight={ROW_HEIGHT} padding={LIST_PADDING} track=".conversation-row"
      keyOf={row => row.id} onKeyDown={onKeyDown}
      renderRow={row => <SessionItem row={row} active={row.id === selectedId} now={now} onClick={() => open(row)} />} />;
  }

  return <div className="session-pane">
    <div className="session-tools">
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label="搜索会话" placeholder="搜索会话" value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      {foldedView && !query.trim()
        ? <div className="folded-head"><button type="button" className="icon-button small" aria-label="返回全部会话" onClick={() => setFoldedView(false)}><ArrowLeft size={16} /></button><strong>折叠的群聊</strong><span className="num">{foldedCount}</span></div>
        : <Segmented className="session-filter" count={FILTERS.length} index={FILTERS.indexOf(filter)} role="group" aria-label="会话类型">
          {FILTERS.map(value => <button type="button" key={value} aria-pressed={filter === value} onClick={() => setFilter(value)}>{filterLabels[value]}</button>)}
        </Segmented>}
      {sample && <p className="session-note">示例数据 · 连接 weflow serve 后显示真实会话</p>}
    </div>
    {body}
  </div>;
}
