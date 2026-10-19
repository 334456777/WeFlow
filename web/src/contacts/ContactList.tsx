// The contacts (#205), by kind with how many of each, or searched across all kinds by name, remark or WeChat ID.
import { useMemo, useState } from 'react';
import { ArrowClockwise, CircleNotch, MagnifyingGlass } from '@phosphor-icons/react';
import { CONTACT_KINDS, contactKindLabels, countKinds, matchContacts, type Contact, type ContactKind } from '../api/contacts';
import { Avatar } from '../Avatar';
import { Segmented } from '../Segmented';
import { VirtualList } from '../VirtualList';
import type { ContactsState } from './useContacts';

const ROW_HEIGHT = 58;

/** Under the name: the nickname when a remark stands in for it, and where the person is. */
export function contactLine(c: Contact): string {
  const line = [c.remark && c.nickname && c.nickname !== c.displayName ? c.nickname : '', c.region ?? ''].filter(Boolean).join(' · ');
  return line || (c.type === 'official' ? c.detailDescription ?? '' : '') || contactKindLabels[c.type];
}

export function ContactList({ state, kind, onKind, selected, onSelect, onRetry }: {
  state: ContactsState;
  kind: ContactKind;
  onKind(kind: ContactKind): void;
  selected: string | null;
  onSelect(username: string): void;
  onRetry(): void;
}) {
  const [query, setQuery] = useState('');
  const contacts = state.status === 'ready' ? state.contacts : [];
  const counts = useMemo(() => countKinds(contacts), [contacts]);
  const searching = Boolean(query.trim());
  const rows = useMemo(() => (searching ? matchContacts(contacts, query) : contacts.filter(c => c.type === kind)), [contacts, kind, query, searching]);

  let body;
  if (state.status === 'loading' || state.status === 'idle') {
    body = <p className="session-message" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在读取通讯录…</p>;
  } else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />重试</button></div>;
  } else if (!rows.length) {
    body = <p className="session-message">{searching ? '没有匹配的联系人。' : '没有' + contactKindLabels[kind] + '。'}</p>;
  } else {
    body = <VirtualList className="conversation-list scroll-area" aria-label={searching ? '搜索到的联系人' : contactKindLabels[kind]} items={rows} rowHeight={ROW_HEIGHT} padding={8} track=".conversation-row"
      keyOf={c => c.username} renderRow={c => {
        const active = c.username === selected;
        return <button type="button" className={'conversation-row' + (active ? ' active' : '')} aria-pressed={active} aria-label={'查看' + c.displayName} onClick={() => onSelect(c.username)}>
          <Avatar name={c.displayName} url={c.avatarUrl} />
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{c.displayName}</span>{searching && <span className="conversation-time">{contactKindLabels[c.type]}</span>}</span>
            <span className="conversation-preview clamp-1">{contactLine(c)}</span>
          </span>
        </button>;
      }} />;
  }

  return <div className="session-pane">
    <div className="session-tools">
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label="搜索联系人" placeholder="搜索名称、备注或微信号" value={query} onChange={event => setQuery(event.target.value)} />
      </div>
      <Segmented className="session-filter contact-kinds" count={CONTACT_KINDS.length} index={CONTACT_KINDS.indexOf(kind)} role="group" aria-label="联系人类型">
        {CONTACT_KINDS.map(value => <button type="button" key={value} aria-pressed={kind === value} aria-label={contactKindLabels[value] + ' ' + counts[value] + ' 个'}
          onClick={() => { onKind(value); setQuery(''); }}>{value === 'former_friend' ? '曾经' : contactKindLabels[value]}{state.status === 'ready' && <small className="num">{counts[value]}</small>}</button>)}
      </Segmented>
    </div>
    {body}
  </div>;
}
