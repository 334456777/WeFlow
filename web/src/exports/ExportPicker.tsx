// The conversations to export (#211): searched, narrowed by kind, picked one by one or all those shown at once.
import { useMemo, useState } from 'react';
import { ArrowClockwise, Check, MagnifyingGlass, X } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { Avatar } from '../Avatar';
import { filterLabels, kindLabels, kindOf, type SessionFilter } from '../chat/sessions';
import type { SessionsState } from '../chat/useSessions';
import { Segmented } from '../Segmented';
import { VirtualList } from '../VirtualList';
import { ListSkeleton } from '../Skeleton';
import { locale, t } from '../i18n';

const ROW_HEIGHT = 58;
const FILTERS: SessionFilter[] = ['all', 'private', 'group', 'official'];

export const nameOf = (session: ChatSession) => session.displayName || session.username;

/** The conversations of `filter` whose name or id holds `query`. */
export function pickable(sessions: readonly ChatSession[], filter: SessionFilter, query: string): ChatSession[] {
  const needle = query.trim().toLocaleLowerCase();
  return sessions.filter(s => (filter === 'all' || kindOf(s.username) === filter)
    && (!needle || nameOf(s).toLocaleLowerCase().includes(needle) || s.username.toLocaleLowerCase().includes(needle)));
}

/** Beside the list: the conversations picked, each one to take off again. */
export function PickedList({ picked, onRemove }: { picked: ChatSession[]; onRemove(id: string): void }) {
  if (!picked.length) return <div className="pane-message"><p>{t('在左侧勾选要导出的会话，可以按类型筛选后全选。')}</p></div>;
  return <div className="picked" aria-label={t('已选的会话')}>
    <h2>{t('已选 {length} 个会话', { length: picked.length.toLocaleString(locale()) })}</h2>
    <ul>
      {picked.map(s => <li key={s.username}>
        <Avatar name={nameOf(s)} url={s.avatarUrl || undefined} />
        <span className="picked-copy"><strong className="clamp-1">{nameOf(s)}</strong><small>{t(kindLabels[kindOf(s.username)])}{s.messageCountHint ? ' · ' + t('约 {n} 条', { n: s.messageCountHint.toLocaleString(locale()) }) : ''}</small></span>
        <button type="button" className="icon-button small" aria-label={t('不导出{name}', { name: nameOf(s) })} onClick={() => onRemove(s.username)}><X size={14} /></button>
      </li>)}
    </ul>
  </div>;
}

export function ExportPicker({ state, picked, onChange, onRetry }: {
  state: SessionsState;
  picked: ReadonlySet<string>;
  onChange(picked: Set<string>): void;
  onRetry(): void;
}) {
  const [filter, setFilter] = useState<SessionFilter>('all');
  const [query, setQuery] = useState('');
  const shown = useMemo(() => (state.status === 'ready' ? pickable(state.sessions, filter, query) : []), [state, filter, query]);
  const allShown = shown.length > 0 && shown.every(s => picked.has(s.username));
  const toggle = (id: string) => {
    const next = new Set(picked);
    if (next.has(id)) next.delete(id); else next.add(id);
    onChange(next);
  };
  const toggleShown = () => {
    const next = new Set(picked);
    for (const s of shown) { if (allShown) next.delete(s.username); else next.add(s.username); }
    onChange(next);
  };

  let body;
  if (state.status === 'loading') body = <ListSkeleton label={t('正在读取会话')} />;
  else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  } else if (!shown.length) body = <p className="session-message">{query.trim() ? t('没有名称匹配的会话。') : t('没有这类会话。')}</p>;
  else {
    body = <VirtualList data-roving="vertical" data-roving-items=".pick-row input" className="conversation-list scroll-area" aria-label={t('可导出的会话')} items={shown} rowHeight={ROW_HEIGHT} padding={8} track=".pick-row"
      keyOf={s => s.username} renderRow={s => {
        const on = picked.has(s.username);
        return <label className={'conversation-row pick-row' + (on ? ' active' : '')}>
          <span className="checkbox"><input type="checkbox" aria-label={t('导出{name}', { name: nameOf(s) })} checked={on} onChange={() => toggle(s.username)} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span></span>
          <Avatar name={nameOf(s)} url={s.avatarUrl || undefined} />
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{nameOf(s)}</span></span>
            <span className="conversation-preview clamp-1">{t(kindLabels[kindOf(s.username)])}{s.messageCountHint ? ' · ' + t('约 {n} 条', { n: s.messageCountHint.toLocaleString(locale()) }) : ''}</span>
          </span>
        </label>;
      }} />;
  }

  return <div className="session-pane">
    <div className="session-tools">
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label={t('搜索会话')} placeholder={t('搜索会话')} value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      <Segmented className="session-filter" count={FILTERS.length} index={FILTERS.indexOf(filter)} role="group" aria-label={t('会话类型')}>
        {FILTERS.map(value => <button type="button" key={value} aria-pressed={filter === value} onClick={() => setFilter(value)}>{t(filterLabels[value])}</button>)}
      </Segmented>
      {state.status === 'ready' && <div className="pick-tools">
        <label className="checkbox"><input type="checkbox" aria-label={t('全选当前列出的会话')} checked={allShown} disabled={!shown.length} onChange={toggleShown} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{t('全选当前列出的 {length} 个', { length: shown.length.toLocaleString(locale()) })}</span></label>
        {picked.size > 0 && <button type="button" className="text-button" onClick={() => onChange(new Set())}>{t('清空（已选 {size}）', { size: picked.size })}</button>}
      </div>}
    </div>
    {body}
  </div>;
}
