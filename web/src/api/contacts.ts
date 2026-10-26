// The contacts (#205): `chat contacts --parsed`, the list as the desktop app shows it, read in process (`/api/`).
import { get } from './client';
import { keptRead } from './kept';

export type ContactKind = 'friend' | 'group' | 'official' | 'former_friend';

export const CONTACT_KINDS: ContactKind[] = ['friend', 'group', 'official', 'former_friend'];
export const contactKindLabels: Record<ContactKind, string> = { friend: '好友', group: '群聊', official: '公众号', former_friend: '曾经的好友' };

/** A row of `chat contacts --parsed`, the most recent conversation first. */
export interface Contact {
  username: string;
  /** The remark, else the nickname, else the WeChat ID, else the username. */
  displayName: string;
  type: ContactKind;
  remark?: string;
  nickname?: string;
  /** The WeChat ID the person chose. */
  alias?: string;
  labels?: string[];
  region?: string;
  /** The signature. */
  detailDescription?: string;
  avatarUrl?: string;
}

export async function getContacts(signal?: AbortSignal): Promise<Contact[]> {
  const data = await get<unknown>('api/chat/contacts', { parsed: true }, signal);
  return Array.isArray(data) ? data.filter((c): c is Contact => typeof c?.username === 'string' && typeof c?.type === 'string') : [];
}

/**
 * The parsed contact list for every page that names people by it, the contacts page and the footprint (#402): read once
 * for the server and account (`keptRead`); `fresh` reads it again, as a retry does.
 */
export const contactList = (fresh = false) => keptRead('contacts', () => getContacts(), fresh);

/** The contacts whose names, WeChat ID or username hold `query`, ignoring case. */
export function matchContacts(contacts: readonly Contact[], query: string): Contact[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [...contacts];
  return contacts.filter(c => [c.displayName, c.remark, c.nickname, c.alias, c.username]
    .some(v => v?.toLocaleLowerCase().includes(needle)));
}

export function countKinds(contacts: readonly Contact[]): Record<ContactKind, number> {
  const counts: Record<ContactKind, number> = { friend: 0, group: 0, official: 0, former_friend: 0 };
  for (const c of contacts) if (c.type in counts) counts[c.type] += 1;
  return counts;
}

/** The formats of `export contacts --format`. */
export const CONTACT_FORMATS = [
  { value: 'excel', label: 'Excel' }, { value: 'csv', label: 'CSV' }, { value: 'json', label: 'JSON' }, { value: 'html', label: 'HTML' },
  { value: 'txt', label: 'TXT' }, { value: 'sql', label: 'SQL' }, { value: 'chatlab', label: 'ChatLab' }, { value: 'weclone', label: 'WeClone' },
] as const;
export type ContactFormat = typeof CONTACT_FORMATS[number]['value'];
