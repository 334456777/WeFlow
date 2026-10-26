// The console's conversation details, members and media (#200).
import { ApiError, get, type Params } from './client';
import type * as Contract from './contract.gen';
import type { ParsedMessage } from './messages';

// `chat detail --extra` starts a command, and unlike the `/api/` reads it does not wait for a place: with the server's
// reads (`--max-reads`, one per processor by default) all taken, it answers `busy` (429) at once. Read such routes one
// at a time, so that the open cards do not crowd each other out; other tabs may still take the places, so retry
// `busy` briefly.
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

/** `chat detail --fast`: who it is and how many messages (in the server's process; the contract's, #430). */
export type SessionDetail = Contract.SessionFacts;

/** `chat detail --extra`: the first and latest message, in Unix seconds, and where the messages are kept. */
export type SessionDetailExtra = Contract.SessionTables;

export async function getDetail(sessionId: string, signal?: AbortSignal): Promise<SessionDetail> {
  const data = await get<SessionDetail | null>('api/chat/detail', { 'session-id': sessionId, fast: true }, signal);
  return { wxid: sessionId, displayName: '', messageCount: 0, ...data };
}

/** Runs as a command of its own (not in the server's process): asked for after the fast part. */
export async function getDetailExtra(sessionId: string, signal?: AbortSignal): Promise<SessionDetailExtra> {
  const data = await commandRead<SessionDetailExtra | null>('chat/detail', { 'session-id': sessionId, extra: true }, signal);
  return { firstMessageTime: null, latestMessageTime: null, messageTables: [], ...data };
}

/** A member of the members panel (`group members`, the contract's). */
export type PanelMember = Contract.GroupMember;

export async function getPanel(chatroomId: string, signal?: AbortSignal): Promise<PanelMember[]> {
  const data = await get<Contract.GroupMembers | null>('api/group/members', { 'chatroom-id': chatroomId }, signal);
  return data?.members ?? [];
}

/** `group member`: one member's messages in the group. */
export type MemberStats = Contract.MemberAnalytics;

export const getMemberStats = (chatroomId: string, username: string, signal?: AbortSignal) =>
  get<MemberStats>('api/group/member', { 'chatroom-id': chatroomId, username }, signal);

/** What a message of `chat resources` holds. */
export type ResourceType = Contract.ResourceKind;

/** A message of `chat resources` (the contract's, #399): its `_cursor` is its place in the listing, not in its
 * conversation; `lookupMessage` finds that. */
export type Resource = Contract.Resource;

export type ResourcePage = Contract.ResourcePage;

export const RESOURCE_PAGE = 60;

/** The conversation's images, videos and files, newest first: from the newest, or from the `nextCursor` of a page. */
export function getResources(sessionId: string, types: readonly ResourceType[], before: string | null, signal?: AbortSignal): Promise<ResourcePage> {
  return listResources({ session: sessionId, types, from: '', to: '' }, before, RESOURCE_PAGE, signal);
}

/** One message by its local id and time, parsed as a page's and with its `_cursor`: to open its conversation there. */
export function lookupMessage(sessionId: string, localId: number, createTime: number, signal?: AbortSignal): Promise<ParsedMessage | null> {
  return get<ParsedMessage | null>('api/chat/message', { 'session-id': sessionId, 'local-id': localId, 'create-time': createTime, parsed: true }, signal);
}

/** What the resource browser (#206) asks for: of one conversation or all, of some types, sent between two days. */
export interface ResourceQuery {
  session: string | null;
  types: readonly ResourceType[];
  /** `YYYY-MM-DD`, inclusive; empty for no bound. */
  from: string;
  to: string;
}

/** A page of `chat resources` across conversations, from the newest or from the `nextCursor` of a page. */
export async function listResources(query: ResourceQuery, before: string | null, limit: number, signal?: AbortSignal): Promise<ResourcePage> {
  const data = await get<Partial<ResourcePage> | null>('api/chat/resources', {
    session: query.session, type: [...query.types], start: query.from, end: query.to, limit, before,
  }, signal);
  const nextCursor = data?.hasMore ? data.nextCursor ?? null : null;
  return { items: data?.items ?? [], hasMore: nextCursor !== null, nextCursor };
}

/** A voice message's length in seconds, from the `voicelength` (milliseconds) of its XML. */
export function voiceSeconds(item: Resource): number | undefined {
  const ms = /voicelength\s*=\s*"(\d+)"/.exec(item.content ?? '')?.[1];
  return ms ? Math.round(Number(ms) / 1000) : undefined;
}
