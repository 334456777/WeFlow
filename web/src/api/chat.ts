// The chat reads of `weflow serve` (docs/serve-api.md): the conversation list and the folded and muted states, read in
// the server's process (`/api/`).
import { ApiError, get } from './client';

/** A row of `chat sessions`, newest first. */
export interface ChatSession {
  /** The other party's wxid, `xxx@chatroom` for a group, `gh_…` for an official account. */
  username: string;
  displayName: string;
  summary: string;
  unreadCount: number;
  /** Unix seconds; 0 for an official account WeChat lists without a conversation. */
  sortTimestamp: number;
  lastTimestamp: number;
  lastMsgType: number;
  avatarUrl?: string;
  /** In a group: who sent the last message. */
  lastMsgSender?: string;
  lastSenderDisplayName?: string;
  messageCountHint?: number;
  selfWxid?: string;
}

export interface SessionStatus {
  isFolded: boolean;
  isMuted: boolean;
}

const SESSION_FIELDS = [
  'username', 'displayName', 'summary', 'unreadCount', 'sortTimestamp', 'lastTimestamp', 'lastMsgType',
  'avatarUrl', 'lastMsgSender', 'lastSenderDisplayName', 'messageCountHint', 'selfWxid',
];

export async function getSessions(signal?: AbortSignal): Promise<ChatSession[]> {
  const data = await get<unknown>('api/chat/sessions', { fields: SESSION_FIELDS.join(',') }, signal);
  if (!Array.isArray(data)) throw new ApiError(200, 'bad_response', '会话列表的格式不对');
  return data as ChatSession[];
}

/** Usernames per request: the list goes in the query, so it is kept to a few kilobytes. */
const STATUS_BATCH = 100;

/**
 * The folded and muted state of every session, a batch after another. An `/api/` read waits in line for a place,
 * and answers `busy` only when it waited too long: such a batch is tried again.
 */
export async function getStatuses(usernames: readonly string[], signal?: AbortSignal): Promise<Record<string, SessionStatus>> {
  const all: Record<string, SessionStatus> = {};
  for (let i = 0; i < usernames.length; i += STATUS_BATCH) {
    const batch = usernames.slice(i, i + STATUS_BATCH);
    for (let attempt = 0; ; attempt++) {
      try {
        Object.assign(all, await get<Record<string, SessionStatus>>('api/chat/statuses', { usernames: batch }, signal));
        break;
      } catch (problem) {
        if (!(problem instanceof ApiError && problem.code === 'busy') || attempt >= 2) throw problem;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  }
  return all;
}
