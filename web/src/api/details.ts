// 操作控制台里的会话详情、成员和媒体（#200）。
import { ApiError, get, type Params } from './client';
import type * as Contract from './contract.gen';
import type { ParsedMessage } from './messages';

// `chat detail --extra` 要启动一个命令，不像 `/api/` 的读取那样排队等待空位：服务端的读取名额（`--max-reads`，
// 默认每个处理器一个）都被占用时，它立即回答 `busy`（429）。这类路由一次只读一个，打开的几张卡片才不会互相抢占名额；
// 其他标签页仍可能占用名额，所以遇到 `busy` 时短暂重试。
let commands: Promise<unknown> = Promise.resolve();
function commandRead<T>(path: string, params: Params, signal?: AbortSignal): Promise<T> {
  const next = commands.then(async () => {
    for (let attempt = 0; ; attempt++) {
      signal?.throwIfAborted();
      try { return await get<T>(path, params, signal); }
      catch (problem) {
        if (!(problem instanceof ApiError && problem.code === 'busy') || attempt >= 2) throw problem;
        await new Promise<void>((resolve, reject) => {
          const abort = () => { clearTimeout(timer); reject(signal?.reason); };
          const timer = setTimeout(() => { signal?.removeEventListener('abort', abort); resolve(); }, 1000);
          signal?.addEventListener('abort', abort, { once: true });
          if (signal?.aborted) abort();
        });
      }
    }
  });
  commands = next.catch(() => {});
  return next;
}

/** `chat detail --fast`：会话的对象和消息数（在服务进程内读取；契约的类型，#430）。 */
export type SessionDetail = Contract.SessionFacts;

/** `chat detail --extra`：第一条和最新一条消息的时间（Unix 秒数），以及消息保存在哪些表里。 */
export type SessionDetailExtra = Contract.SessionTables;

/** `chat detail --fast`，在服务进程内读取；服务端没给的字段当作空。 */
export async function getDetail(sessionId: string, signal?: AbortSignal): Promise<SessionDetail> {
  const data = await get<SessionDetail | null>('api/chat/detail', { 'session-id': sessionId, fast: true }, signal);
  return { wxid: sessionId, displayName: '', messageCount: 0, ...data };
}

/** `chat detail --extra`：作为单独的命令运行（不在服务进程内），在快速的部分之后再请求。 */
export async function getDetailExtra(sessionId: string, signal?: AbortSignal): Promise<SessionDetailExtra> {
  const data = await commandRead<SessionDetailExtra | null>('chat/detail', { 'session-id': sessionId, extra: true }, signal);
  return { firstMessageTime: null, latestMessageTime: null, messageTables: [], ...data };
}

/** `group member`：一个成员在群里的消息统计。 */
export type MemberStats = Contract.MemberAnalytics;

/** `group member`，在服务进程内读取。 */
export const getMemberStats = (chatroomId: string, username: string, signal?: AbortSignal) =>
  get<MemberStats>('api/group/member', { 'chatroom-id': chatroomId, username }, signal);

/** `chat resources` 的一条消息里的内容种类。 */
export type ResourceType = Contract.ResourceKind;

/** `chat resources` 的一条消息（契约的类型，#399）：它的 `_cursor` 是在这个列表里的位置，不是在会话里的位置；
 * 会话里的位置用 `lookupMessage` 查找。 */
export type Resource = Contract.Resource;

/** `chat resources` 的一页。 */
export type ResourcePage = Contract.ResourcePage;

/** 资源每页的消息数。 */
export const RESOURCE_PAGE = 60;

/** 会话里的图片、视频和文件，最新的在前：从最新的开始，或从某一页的 `nextCursor` 开始。 */
export function getResources(sessionId: string, types: readonly ResourceType[], before: string | null, signal?: AbortSignal): Promise<ResourcePage> {
  return listResources({ session: sessionId, types, from: '', to: '' }, before, RESOURCE_PAGE, signal);
}

/** 按本地 ID 和时间查找一条消息，与页面读到的一样是解析过的、带 `_cursor`：用来在这个位置打开它的会话。 */
export function lookupMessage(sessionId: string, localId: number, createTime: number, signal?: AbortSignal): Promise<ParsedMessage | null> {
  return get<ParsedMessage | null>('api/chat/message', { 'session-id': sessionId, 'local-id': localId, 'create-time': createTime, parsed: true }, signal);
}

/** 资源浏览（#206）的查询条件：一个会话或全部会话、哪些类型、在哪两天之间发送。 */
export interface ResourceQuery {
  session: string | null;
  types: readonly ResourceType[];
  /** `YYYY-MM-DD`，包含这一天；为空表示不限。 */
  from: string;
  to: string;
}

/** 跨会话的 `chat resources` 的一页：从最新的开始，或从某一页的 `nextCursor` 开始。 */
export async function listResources(query: ResourceQuery, before: string | null, limit: number, signal?: AbortSignal): Promise<ResourcePage> {
  const data = await get<Partial<ResourcePage> | null>('api/chat/resources', {
    session: query.session, type: [...query.types], start: query.from, end: query.to, limit, before,
  }, signal);
  const nextCursor = data?.hasMore ? data.nextCursor ?? null : null;
  return { items: data?.items ?? [], hasMore: nextCursor !== null, nextCursor };
}

/** 语音消息的时长（秒），取自 XML 里的 `voicelength`（毫秒）。 */
export function voiceSeconds(item: Resource): number | undefined {
  const ms = /voicelength\s*=\s*"(\d+)"/.exec(item.content ?? '')?.[1];
  return ms ? Math.round(Number(ms) / 1000) : undefined;
}
