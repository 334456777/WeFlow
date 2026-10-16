// The console's conversation details, members and media (#200).
import { ApiError, get, type Params } from './client';
import type { GroupMember, ParsedMessage } from './messages';

// These routes each start a command, and unlike the `/api/` reads they do not wait for a place: with the server's
// reads (`--max-reads`, one per processor by default) all taken, they answer `busy` (429) at once. Read them one at
// a time, so that the open cards do not crowd each other out; other tabs may still take the places, so retry `busy`
// briefly.
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

/** `chat detail --fast`: who it is and how many messages (in the server's process). */
export interface SessionDetail {
  wxid: string;
  displayName: string;
  remark?: string;
  nickName?: string;
  alias?: string;
  avatarUrl?: string;
  messageCount: number;
}

/** `chat detail --extra`: the first and latest message, in Unix seconds, and where the messages are kept. */
export interface SessionDetailExtra {
  firstMessageTime: number | null;
  latestMessageTime: number | null;
  messageTables: { dbName: string; tableName: string; count: number }[];
}

export async function getDetail(sessionId: string, signal?: AbortSignal): Promise<SessionDetail> {
  const data = await get<SessionDetail | null>('api/chat/detail', { 'session-id': sessionId, fast: true }, signal);
  return { wxid: sessionId, displayName: '', messageCount: 0, ...data };
}

/** Runs as a command of its own (not in the server's process): asked for after the fast part. */
export async function getDetailExtra(sessionId: string, signal?: AbortSignal): Promise<SessionDetailExtra> {
  const data = await commandRead<SessionDetailExtra | null>('chat/detail', { 'session-id': sessionId, extra: true }, signal);
  return { firstMessageTime: null, latestMessageTime: null, messageTables: [], ...data };
}

/** A member of the members panel (`group members`). */
export interface PanelMember extends GroupMember {
  nickname?: string;
  remark?: string;
  alias?: string;
  isOwner?: boolean;
  isFriend?: boolean;
}

export async function getPanel(chatroomId: string, signal?: AbortSignal): Promise<PanelMember[]> {
  const data = await get<{ members?: PanelMember[] }>('api/group/members', { 'chatroom-id': chatroomId }, signal);
  return data?.members ?? [];
}

/** `group member`: one member's messages in the group. */
export interface MemberStats {
  statistics: {
    totalMessages: number;
    textMessages: number;
    imageMessages: number;
    voiceMessages: number;
    videoMessages: number;
    emojiMessages: number;
    otherMessages: number;
    firstMessageTime: number | null;
    lastMessageTime: number | null;
    activeDays: number;
  };
  timeDistribution: Record<string, number>;
  commonPhrases: { phrase: string; count: number }[];
  commonEmojis: { emoji: string; count: number }[];
}

export function getMemberStats(chatroomId: string, username: string, signal?: AbortSignal): Promise<MemberStats> {
  return commandRead<MemberStats>('group/member', { 'chatroom-id': chatroomId, username }, signal);
}

export type ResourceType = 'image' | 'video' | 'file';

/** A message of `chat resources`: no `_cursor`; `lookupMessage` finds its place. */
export interface Resource {
  localId: number;
  createTime: number;
  localType: number;
  resourceType: ResourceType | 'voice';
  senderUsername?: string | null;
  isSend?: number | null;
  imageMd5?: string;
  videoMd5?: string;
  fileName?: string;
  fileSize?: number;
  fileExt?: string;
  parsedContent?: string;
}

export const RESOURCE_PAGE = 60;

/** The conversation's images, videos and files, newest first. */
export async function getResources(sessionId: string, types: readonly ResourceType[], offset: number, signal?: AbortSignal): Promise<{ items: Resource[]; hasMore: boolean }> {
  const data = await commandRead<{ items?: Resource[]; hasMore?: boolean }>('chat/resources', { session: sessionId, type: [...types], limit: RESOURCE_PAGE, offset }, signal);
  return { items: data?.items ?? [], hasMore: Boolean(data?.hasMore) };
}

/** One message by its local id and time, parsed as a page's and with its `_cursor`: to open its conversation there. */
export function lookupMessage(sessionId: string, localId: number, createTime: number, signal?: AbortSignal): Promise<ParsedMessage | null> {
  return get<ParsedMessage | null>('api/chat/message', { 'session-id': sessionId, 'local-id': localId, 'create-time': createTime, parsed: true }, signal);
}
