// The chat reads of `weflow serve` (docs/serve-api.md): the conversation list and the folded and muted states, read in
// the server's process (`/api/`).
import { ApiError, get } from './client';
import type * as Contract from './contract.gen';

/** Whether a conversation is folded and muted (the contract's `ChatStatus`, #430). */
export type SessionStatus = Contract.ChatStatus;

const SESSION_KEYS = [
  'username', 'displayName', 'summary', 'unreadCount', 'sortTimestamp', 'lastTimestamp', 'lastMsgType',
  'avatarUrl', 'lastMsgSender', 'lastSenderDisplayName', 'messageCountHint', 'selfWxid',
] as const satisfies readonly (keyof Contract.Session)[];

/**
 * A row of `chat sessions` (the contract's, #399), newest first; only the keys the list uses are asked for. The page
 * makes rows of its own too (the samples, official accounts without a conversation), without `selfWxid`.
 */
export type ChatSession = Pick<Contract.Session, Exclude<(typeof SESSION_KEYS)[number], 'selfWxid'>> &
  Partial<Pick<Contract.Session, 'selfWxid'>>;

export async function getSessions(signal?: AbortSignal): Promise<ChatSession[]> {
  const data = await get<unknown>('api/chat/sessions', { fields: SESSION_KEYS.join(',') }, signal);
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
        Object.assign(all, await get<Contract.Answers['chat statuses']>('api/chat/statuses', { usernames: batch }, signal));
        break;
      } catch (problem) {
        if (!(problem instanceof ApiError && problem.code === 'busy') || attempt >= 2) throw problem;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  }
  return all;
}
