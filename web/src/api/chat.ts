// `weflow serve` 的聊天读取（见 Wiki 的「命令 API」页）：会话列表，以及会话的折叠和免打扰状态，
// 都在服务进程内读取（`/api/`）。
import { ApiError, get } from './client';
import type * as Contract from './contract.gen';
import { t } from '../i18n';

/** 会话是否折叠、是否免打扰（契约的 `ChatStatus`，#430）。 */
export type SessionStatus = Contract.ChatStatus;

const SESSION_KEYS = [
  'username', 'displayName', 'summary', 'unreadCount', 'sortTimestamp', 'lastTimestamp', 'lastMsgType',
  'avatarUrl', 'lastMsgSender', 'lastSenderDisplayName', 'messageCountHint', 'selfWxid',
] as const satisfies readonly (keyof Contract.Session)[];

/**
 * `chat sessions` 的一行（契约的类型，#399），最新的在前；只请求列表用到的键。
 * 网页也会自己生成行（示例数据、没有会话的公众号），这些行没有 `selfWxid`。
 */
export type ChatSession = Pick<Contract.Session, Exclude<(typeof SESSION_KEYS)[number], 'selfWxid'>> &
  Partial<Pick<Contract.Session, 'selfWxid'>>;

/** `chat sessions`：会话列表，只带列表用到的键。 */
export async function getSessions(signal?: AbortSignal): Promise<ChatSession[]> {
  const data = await get<unknown>('api/chat/sessions', { fields: SESSION_KEYS.join(',') }, signal);
  if (!Array.isArray(data)) throw new ApiError(200, 'bad_response', t('会话列表的格式不对'));
  return data as ChatSession[];
}

/** 每个请求带的用户名数：用户名放在查询参数里，所以要控制在几 KB 以内。 */
const STATUS_BATCH = 100;

/**
 * 逐批读取每个会话的折叠和免打扰状态。`/api/` 的读取会排队等待空位，只有等得太久时才回答 `busy`：
 * 这样的一批隔 1 秒重试，最多重试两次。
 */
export async function getStatuses(usernames: readonly string[], signal?: AbortSignal): Promise<Record<string, SessionStatus>> {
  const all: Record<string, SessionStatus> = {};
  for (let i = 0; i < usernames.length; i += STATUS_BATCH) {
    const batch = usernames.slice(i, i + STATUS_BATCH);
    for (let attempt = 0; ; attempt++) {
      try {
        Object.assign(all, await get<Contract.Answers['chat statuses']>('api/chat/statuses', { sessions: batch }, signal));
        break;
      } catch (problem) {
        if (!(problem instanceof ApiError && problem.code === 'busy') || attempt >= 2) throw problem;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  }
  return all;
}
