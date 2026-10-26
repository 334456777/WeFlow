// Who posts in Moments and how often (#202), for the list beside the timeline: the counts of `sns post-counts`,
// named from the contacts. The counts are read each time the page is opened, the contacts once per server and account
// (`keptRead`): the whole table, as some who post are in no list of `chat contacts --parsed`.
import { useEffect, useState } from 'react';
import { ApiError, get } from '../api/client';
import { keptRead } from '../api/kept';
import { getPostCounts } from '../api/sns';

export interface MomentPerson {
  username: string;
  name: string;
  avatarUrl?: string;
  posts: number;
}

export type PeopleState =
  | { status: 'loading' }
  | { status: 'ready'; people: MomentPerson[] }
  | { status: 'error'; message: string };

const text = (row: Record<string, unknown>, ...keys: string[]) => {
  for (const key of keys) {
    const value = row[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
};

/** The posters, most posts first; `self` (this account) is named 「我」. */
export function peopleOf(counts: Record<string, number>, contacts: Record<string, unknown>[], self?: string): MomentPerson[] {
  const known = new Map(contacts.map(row => [text(row, 'username', 'userName'), row]));
  return Object.entries(counts)
    .filter(([, posts]) => posts > 0)
    .map(([username, posts]) => {
      const row = known.get(username);
      const name = username === self ? '我' : (row && text(row, 'remark', 'nick_name', 'nickName', 'alias')) || username;
      const avatarUrl = row ? text(row, 'small_head_url', 'big_head_url', 'avatarUrl') || undefined : undefined;
      return { username, name, avatarUrl, posts };
    })
    .sort((a, b) => b.posts - a.posts || a.name.localeCompare(b.name, 'zh-CN'));
}

export function useMomentPeople(enabled: boolean, scope: number, self?: string): [PeopleState, () => void] {
  const [state, setState] = useState<{ scope: number; value: PeopleState } | null>(null);
  const [round, setRound] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    setState(old => (old?.scope === scope && old.value.status === 'ready' ? old : { scope, value: { status: 'loading' } }));
    Promise.all([getPostCounts(controller.signal), keptRead('contacts:all', () => get<Record<string, unknown>[]>('api/chat/contacts', {}))]).then(
      ([counts, contacts]) => { if (!controller.signal.aborted) setState({ scope, value: { status: 'ready', people: peopleOf(counts, Array.isArray(contacts) ? contacts : [], self) } }); },
      problem => {
        if (controller.signal.aborted) return;
        setState({ scope, value: { status: 'error', message: (problem instanceof ApiError && problem.message) || '读取朋友圈的发布者失败' } });
      },
    );
    return () => controller.abort();
  }, [enabled, scope, self, round]);
  return [state?.scope === scope ? state.value : { status: 'loading' }, () => setRound(n => n + 1)];
}
