// 公众号（#203）：关注的公众号（`biz accounts`），以及一个公众号按文章列出的推送（`biz messages`），
// 都在服务进程内读取（`/api/`，见 Wiki 的「命令 API」页）。
import { get } from './client';
import type * as Contract from './contract.gen';

/** 关注的一个公众号（`biz accounts`）。 */
export interface BizAccount {
  username: string;
  name: string;
  avatarUrl?: string;
  /** 它的会话里最后一条消息的 Unix 秒数；没有消息时为 0。 */
  lastTime: number;
}

/** 推送里的一篇文章，一次推送有一篇或几篇（契约的类型，#430）。 */
export type BizArticle = Contract.BizArticle;

/** 公众号的一次推送：第一篇文章和其余的文章（`contentList`）。 */
export type BizPush = Contract.BizMessage;

/** 每页的推送数。 */
export const BIZ_PAGE = 20;

const text = (row: Record<string, unknown>, ...keys: string[]) => {
  for (const key of keys) {
    const value = row[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
};

/** `biz accounts`：关注的公众号，各带名称和最后一次推送的时间。 */
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

/** `biz messages`：`username` 从第 `offset` 条起的一页推送（最多 `BIZ_PAGE` 条），最新的在前。 */
export async function getBizPushes(username: string, offset: number, signal?: AbortSignal): Promise<BizPush[]> {
  const data = await get<Contract.BizMessages | null>('api/biz/messages', { username, limit: BIZ_PAGE, offset }, signal);
  return data?.messages ?? [];
}
