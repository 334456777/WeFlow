// 谁发了朋友圈、各发了多少条（#202），给时间线旁边的列表用：条数来自 `sns post-counts`，名称取自联系人。
// 条数每次打开页面时读取，联系人每个服务端和账号只读一次（`keptRead`）：读的是整张联系人表，
// 因为有些发布者不在 `chat contacts --parsed` 的任何一类里。
import { useEffect, useState } from 'react';
import { ApiError, get } from '../api/client';
import { t } from '../i18n';
import { keptRead } from '../api/kept';
import { getPostCounts } from '../api/sns';

/** 一个发布者，以及他发的朋友圈条数。 */
export interface MomentPerson {
  username: string;
  name: string;
  avatarUrl?: string;
  posts: number;
}

/** 发布者列表的读取状态：正在读、已读到，或者失败的原因。 */
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

/** 发过朋友圈的人，条数多的在前，条数相同时按名称排；`self`（账号所有者）的名称是 `我`。 */
export function peopleOf(counts: Record<string, number>, contacts: Record<string, unknown>[], self?: string): MomentPerson[] {
  const known = new Map(contacts.map(row => [text(row, 'username', 'userName'), row]));
  return Object.entries(counts)
    .filter(([, posts]) => posts > 0)
    .map(([username, posts]) => {
      const row = known.get(username);
      const name = username === self ? t('我') : (row && text(row, 'remark', 'nick_name', 'nickName', 'alias')) || username;
      const avatarUrl = row ? text(row, 'small_head_url', 'big_head_url', 'avatarUrl') || undefined : undefined;
      return { username, name, avatarUrl, posts };
    })
    .sort((a, b) => b.posts - a.posts || a.name.localeCompare(b.name, 'zh-CN'));
}

/** `enabled` 之后读取的发布者（名称取自联系人），以及重新读取的函数；同一作用域重读时保留已显示的列表。 */
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
        setState({ scope, value: { status: 'error', message: (problem instanceof ApiError && problem.message) || t('读取朋友圈的发布者失败') } });
      },
    );
    return () => controller.abort();
  }, [enabled, scope, self, round]);
  return [state?.scope === scope ? state.value : { status: 'loading' }, () => setRound(n => n + 1)];
}
