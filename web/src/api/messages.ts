// Messages, people and media of a conversation (docs/serve-api.md, "In-process reads"). Messages come parsed by the
// server as the desktop app parses them (`chat messages --parsed`), so the page reads no WeChat XML.
import { apiUrl, get, token, type Params } from './client';

/** A message of `chat messages --parsed`; only the keys the chat window uses are asked for. */
export interface ParsedMessage {
  _cursor: string;
  localId: number;
  /** The server id as text: it does not fit in a JavaScript number. */
  serverId?: string;
  /** WeChat's type; an app message carries its kind in the high bits and is named by `appMsgKind`. */
  localType: number;
  createTime: number;
  isSend: number | null;
  senderUsername: string | null;
  parsedContent: string;
  appMsgKind?: string;
  quotedContent?: string;
  quotedSender?: string;
  linkTitle?: string;
  linkUrl?: string;
  linkThumb?: string;
  appMsgDesc?: string;
  appMsgAppName?: string;
  appMsgSourceName?: string;
  appMsgThumbUrl?: string;
  fileName?: string;
  fileSize?: number;
  fileExt?: string;
  voiceDurationSeconds?: number;
  emojiCdnUrl?: string;
  imageMd5?: string;
  videoMd5?: string;
  cardNickname?: string;
  cardUsername?: string;
  cardAvatarUrl?: string;
  locationPoiname?: string;
  locationLabel?: string;
  finderNickname?: string;
  finderCoverUrl?: string;
  finderDuration?: number;
  chatRecordTitle?: string;
}

const MESSAGE_FIELDS = [
  '_cursor', 'localId', 'serverId', 'localType', 'createTime', 'isSend', 'senderUsername', 'parsedContent', 'appMsgKind',
  'quotedContent', 'quotedSender', 'linkTitle', 'linkUrl', 'linkThumb', 'appMsgDesc', 'appMsgAppName', 'appMsgSourceName',
  'appMsgThumbUrl', 'fileName', 'fileSize', 'fileExt', 'voiceDurationSeconds', 'emojiCdnUrl', 'imageMd5', 'videoMd5',
  'cardNickname', 'cardUsername', 'cardAvatarUrl', 'locationPoiname', 'locationLabel', 'finderNickname', 'finderCoverUrl',
  'finderDuration', 'chatRecordTitle',
].join(',');

export const PAGE_SIZE = 50;

/** Where a page starts: the newest messages, those older or newer than a message's `_cursor`, or a time on. */
export type PageFrom = null | { before: string } | { after: string } | { since: number };

/**
 * A page of messages, newest first: the newest ones, those older than a message's `_cursor` (`before`), the newer
 * ones closest to it (`after`), or the first sent at a time (Unix seconds) or later (`since`, #199).
 */
export function getMessages(sessionId: string, from: PageFrom, signal?: AbortSignal, limit = PAGE_SIZE): Promise<ParsedMessage[]> {
  return get<ParsedMessage[]>('api/chat/messages', {
    'session-id': sessionId, limit, ...(from ?? {}), parsed: true, fields: MESSAGE_FIELDS,
  }, signal);
}

/** A hit of `chat search --parsed`: the message, and the conversation it is in. */
export interface SearchHit extends ParsedMessage {
  _session_id: string;
}

export interface SearchQuery {
  keyword: string;
  /** One conversation, or all of them. */
  sessionId: string | null;
  /** Unix seconds; 0 for no limit. */
  start: number;
  end: number;
}

export const SEARCH_PAGE = 20;

/** Hits newest first, as WeChat's own search matches them; `offset` pages on. */
export function searchMessages(query: SearchQuery, offset: number, signal?: AbortSignal): Promise<SearchHit[]> {
  return get<SearchHit[]>('api/chat/search', {
    keyword: query.keyword, 'session-id': query.sessionId ?? undefined, limit: SEARCH_PAGE, offset,
    start: query.start || undefined, end: query.end || undefined, parsed: true, fields: MESSAGE_FIELDS + ',_session_id',
  }, signal);
}

/** How many messages each day of a conversation has (`YYYY-MM-DD` → count). */
export async function getDateCounts(sessionId: string, signal?: AbortSignal): Promise<Record<string, number>> {
  const data = await get<{ counts?: Record<string, number> }>('api/chat/date-counts', { 'session-id': sessionId }, signal);
  return data?.counts ?? {};
}

export interface GroupMember {
  username: string;
  displayName: string;
  groupNickname?: string;
  avatarUrl?: string;
}

export async function getMembers(chatroomId: string, signal?: AbortSignal): Promise<GroupMember[]> {
  const data = await get<{ members?: GroupMember[] }>('api/group/members', { 'chatroom-id': chatroomId }, signal);
  return data?.members ?? [];
}

export interface Contact {
  username: string;
  remark?: string;
  nick_name?: string;
  small_head_url?: string;
  big_head_url?: string;
}

/** Which files WeChat keeps of a message's image are on this computer, and their sizes (bytes) as the message gives
 * them: the original's (`hd`) even before WeChat has downloaded it (#194). */
export interface ImageInfo { thumb: boolean; normal: boolean; hd: boolean; normalBytes?: number | null; hdBytes?: number | null }

export function getImageInfo(sessionId: string, localId: number, createTime: number, signal?: AbortSignal): Promise<ImageInfo> {
  return get<ImageInfo>('api/chat/image-info', { 'session-id': sessionId, 'msg-id': localId, 'create-time': createTime }, signal);
}

export function getContact(username: string, signal?: AbortSignal): Promise<Contact> {
  return get<Contact>('api/chat/contact', { username }, signal);
}

/**
 * The address of a message's media, for `<img>`, `<audio>` and `<video>`. They cannot send a header, so the token
 * goes in the query when the page has one (the development proxy adds it otherwise).
 */
export function mediaUrl(kind: 'image-data' | 'voice-data' | 'video-data', sessionId: string, message: ParsedMessage, extra: Params = {}): string {
  const bearer = token();
  return apiUrl('api/chat/' + kind, {
    'session-id': sessionId, 'msg-id': message.localId, 'create-time': message.createTime, ...extra,
    ...(bearer ? { access_token: bearer } : {}),
  });
}
