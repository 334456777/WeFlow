// Beside the Moments timeline (#202): everyone's posts, or one poster's, with how many each has; searchable by name.
import { useMemo, useState } from 'react';
import { Aperture, ArrowClockwise, MagnifyingGlass } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { VirtualList } from '../VirtualList';
import type { MomentPerson, PeopleState } from './useMomentPeople';
import { ListSkeleton } from '../Skeleton';

const ROW_HEIGHT = 58;
const ALL = '@all';

export function MomentPeople({ state, total, selected, onSelect, onRetry }: {
  state: PeopleState;
  /** All the posts, from `sns stats`, once known. */
  total: number | null;
  selected: string | null;
  onSelect(username: string | null): void;
  onRetry(): void;
}) {
  const [query, setQuery] = useState('');
  const rows = useMemo(() => {
    if (state.status !== 'ready') return [];
    const needle = query.trim().toLocaleLowerCase();
    const people = needle ? state.people.filter(p => p.name.toLocaleLowerCase().includes(needle) || p.username.toLocaleLowerCase().includes(needle)) : state.people;
    const all: MomentPerson = { username: ALL, name: '全部朋友圈', posts: total ?? state.people.reduce((n, p) => n + p.posts, 0) };
    return needle ? people : [all, ...people];
  }, [state, query, total]);

  let body;
  if (state.status === 'loading') body = <ListSkeleton label="正在读取发布者" />;
  else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />重试</button></div>;
  } else if (!rows.length) body = <p className="session-message">{query.trim() ? '没有名称匹配的发布者。' : '本机没有朋友圈。'}</p>;
  else {
    body = <VirtualList data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list scroll-area" aria-label="朋友圈发布者" items={rows} rowHeight={ROW_HEIGHT} padding={8} track=".conversation-row"
      keyOf={p => p.username} renderRow={p => {
        const id = p.username === ALL ? null : p.username;
        const active = id === selected;
        return <button type="button" className={'conversation-row' + (active ? ' active' : '')} aria-pressed={active} aria-label={(id ? '只看' + p.name : '全部朋友圈') + '，' + p.posts + ' 条'} onClick={() => onSelect(id)}>
          {id ? <Avatar name={p.name} url={p.avatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true"><Aperture size={20} /></span>}
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{p.name}</span></span>
            <span className="conversation-preview clamp-1">{p.posts} 条朋友圈</span>
          </span>
        </button>;
      }} />;
  }

  return <div className="session-pane">
    <div className="session-tools">
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label="搜索发布者" placeholder="搜索发布者" value={query} onChange={event => setQuery(event.target.value)} />
      </div>
    </div>
    {body}
  </div>;
}
