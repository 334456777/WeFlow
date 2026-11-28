// 会话的消息、成员和媒体（见 Wiki 的「命令 API」页）。消息由服务端按桌面版的方式解析好（`chat messages --parsed`），
// 网页不读微信的 XML。
import { get, mediaAddress, token, type Params } from './client';
import type * as Contract from './contract.gen';
import { forgetKept, keptRead } from './kept';
import { onLive } from './live';

const MESSAGE_KEYS = [
  'localId', 'serverId', 'localType', 'createTime', 'isSend', 'senderUsername', 'parsedContent', 'appMsgKind',
  'quotedContent', 'quotedSender', 'linkTitle', 'linkUrl', 'linkThumb', 'appMsgDesc', 'appMsgAppName', 'appMsgSourceName',
  'appMsgThumbUrl', 'fileName', 'fileSize', 'fileExt', 'voiceDurationSeconds', 'emojiCdnUrl', 'imageMd5', 'videoMd5',
  'cardNickname', 'cardUsername', 'cardAvatarUrl', 'locationPoiname', 'locationLabel', 'finderNickname', 'finderCoverUrl',
  'finderDuration', 'chatRecordTitle',
] as const satisfies readonly (keyof Contract.ParsedMessage)[];

/** `chat messages --parsed` 的一条消息（契约的类型，#399）；只请求聊天窗口用到的键。 */
export type ParsedMessage = Pick<Contract.ParsedMessage, (typeof MESSAGE_KEYS)[number]> & { _cursor: string };

/** 消息的基本类型：微信把 app 消息的子类型存在类型的高 32 位，低 32 位才是类型本身。 */
export const baseType = (localType: number) => localType % 4_294_967_296;

const MESSAGE_FIELDS = ['_cursor', ...MESSAGE_KEYS].join(',');

/** 会话每页的消息数。 */
export const PAGE_SIZE = 50;

/** 一页从哪里开始：最新的消息、比某条消息的 `_cursor` 更早或更新的消息，或者某个时间以后的消息。 */
export type PageFrom = null | { before: string } | { after: string } | { since: number };

/**
 * 一页消息，最新的在前：最新的消息、比某条消息的 `_cursor` 更早的消息（`before`）、
 * 比它更新的消息中离它最近的那些（`after`），或者在某个时间（Unix 秒数）或之后最先发送的消息（`since`，#199）。
 */
export function getMessages(sessionId: string, from: PageFrom, signal?: AbortSignal, limit = PAGE_SIZE): Promise<ParsedMessage[]> {
  return get<ParsedMessage[]>('api/chat/messages', {
    'session-id': sessionId, limit, ...(from ?? {}), parsed: true, fields: MESSAGE_FIELDS,
  }, signal);
}

/** `chat search --parsed` 的一条结果：消息，以及它所在的会话。 */
export type SearchHit = ParsedMessage & { _session_id: string };

/** `chat search` 的查询条件。 */
export interface SearchQuery {
  keyword: string;
  /** 只搜这个会话；为 null 时搜全部会话。 */
  sessionId: string | null;
  /** Unix 秒数；0 表示不限。 */
  start: number;
  end: number;
}

/** 搜索每页的结果数。 */
export const SEARCH_PAGE = 20;

/** `chat search --parsed`：按微信自己的搜索方式匹配的结果，最新的在前；用 `offset` 翻页。 */
export function searchMessages(query: SearchQuery, offset: number, signal?: AbortSignal): Promise<SearchHit[]> {
  return get<SearchHit[]>('api/chat/search', {
    keyword: query.keyword, session: query.sessionId ?? undefined, limit: SEARCH_PAGE, offset,
    start: query.start || undefined, end: query.end || undefined, parsed: true, fields: MESSAGE_FIELDS + ',_session_id',
  }, signal);
}

/** 会话每天的消息数（`YYYY-MM-DD` → 条数）。 */
export async function getDateCounts(sessionId: string, signal?: AbortSignal): Promise<Record<string, number>> {
  const data = await get<Contract.SessionDateCounts | null>('api/chat/date-counts', { 'session-id': sessionId }, signal);
  return data?.counts ?? {};
}

/** 群的一个成员（`group members`，契约的类型）。 */
export type GroupMember = Contract.GroupMember;

const membersKey = (chatroomId: string) => 'members:' + chatroomId;

/**
 * 群的成员（`keptRead`）：消息里的人名和会话详情里的成员同时请求时共用一个请求，失败不保留。
 * 离开的调用方只是不使用这个回答，因为别的调用方可能还在等它。成员保留到这个群来了实时消息（可能有人进群或退群），
 * 或者事件流中断后恢复（#201）为止：之后的第一个请求重新读取。
 */
export function getMembers(chatroomId: string): Promise<GroupMember[]> {
  return keptRead(membersKey(chatroomId), async () => {
    const data = await get<Contract.GroupMembers | null>('api/group/members', { 'chatroom-id': chatroomId });
    return data?.members ?? [];
  });
}
onLive(event => {
  if (event.type === 'resumed') forgetKept(key => key.startsWith(membersKey('')));
  else forgetKept(key => key === membersKey(event.sessionId));
});

/**
 * 微信为一条图片消息保存的各个文件中，哪些在服务端所在的电脑上，以及消息里记录的大小（字节）：
 * 原图（`hd`）在微信下载它之前也有大小（#194；契约的类型，#430）。
 */
export type ImageInfo = Contract.ImageInfo;

/** 一条消息的 `chat image-info`，在服务进程内读取。 */
export function getImageInfo(sessionId: string, localId: number, createTime: number, signal?: AbortSignal): Promise<ImageInfo> {
  return get<ImageInfo>('api/chat/image-info', { 'session-id': sessionId, 'local-id': localId, 'create-time': createTime }, signal);
}

/**
 * 消息里媒体的地址，供 `<img>`、`<audio>` 和 `<video>` 使用。它们不能带请求头，
 * 所以页面有 token 时放在查询参数里，没有时由开发服务器的代理加上。
 */
export function mediaUrl(kind: 'image-data' | 'voice-data' | 'video-data', sessionId: string, message: ParsedMessage, extra: Params = {}): string {
  const bearer = token();
  return mediaAddress('api/chat/' + kind, {
    'session-id': sessionId, 'local-id': message.localId, 'create-time': message.createTime, ...extra,
    ...(bearer ? { access_token: bearer } : {}),
  });
}
