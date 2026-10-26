// Official accounts (#203): the ones followed (`biz accounts`), and an account's pushes as articles
// (`biz messages`), both read in the server's process (`/api/`, docs/serve-api.md).
import { get } from './client';

export interface BizAccount {
  username: string;
  name: string;
  avatarUrl?: string;
  /** Unix seconds of its conversation's last message; 0 without one. */
  lastTime: number;
}

/** One article of a push: a push holds one or several. */
export interface BizArticle {
  title: string;
  url: string;
  cover: string;
  summary: string;
}

/** A push of an official account, its first article and the others (`contentList`). */
export interface BizPush {
  localId: number;
  createTime: number;
  title: string;
  description: string;
  url: string;
  cover: string;
  contentList: BizArticle[];
}

/** Pushes per page. */
export const BIZ_PAGE = 20;

const text = (row: Record<string, unknown>, ...keys: string[]) => {
  for (const key of keys) {
    const value = row[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
};

export async function getBizAccounts(signal?: AbortSignal): Promise<BizAccount[]> {
  const data = await get<{ accounts?: Record<string, unknown>[] }>('api/biz/accounts', {}, signal);
  return (data?.accounts ?? []).flatMap(row => {
    const username = text(row, 'username', 'userName');
    if (!username) return [];
    const lastTime = Number(row.lastTime);
    return [{
      username,
      name: text(row, 'remark', 'nick_name', 'nickName', 'alias') || username,
      avatarUrl: text(row, 'big_head_url', 'small_head_url', 'avatarUrl') || undefined,
      lastTime: Number.isFinite(lastTime) ? lastTime : 0,
    }];
  });
}

/** Pushes of `username`, newest first, from `offset` on. */
export async function getBizPushes(username: string, offset: number, signal?: AbortSignal): Promise<BizPush[]> {
  const data = await get<{ messages?: BizPush[] }>('api/biz/messages', { username, limit: BIZ_PAGE, offset }, signal);
  return data?.messages ?? [];
}
