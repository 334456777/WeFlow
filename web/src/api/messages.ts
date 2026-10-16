// Messages, people and media of a conversation (docs/serve-api.md, "In-process reads"). Messages come parsed by the
// server as the desktop app parses them (`chat messages --parsed`), so the page reads no WeChat XML.
import { apiUrl, get, token, type Params } from './client';

/** A message of `chat messages --parsed`; only the keys the chat window uses are asked for. */
export interface ParsedMessage {
  _cursor: string;
  localId: number;
  /** The server id as text: it does not fit in a JavaScript number. */
  serverIdRaw?: string;
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
  '_cursor', 'localId', 'serverIdRaw', 'localType', 'createTime', 'isSend', 'senderUsername', 'parsedContent', 'appMsgKind',
  'quotedContent', 'quotedSender', 'linkTitle', 'linkUrl', 'linkThumb', 'appMsgDesc', 'appMsgAppName', 'appMsgSourceName',
  'appMsgThumbUrl', 'fileName', 'fileSize', 'fileExt', 'voiceDurationSeconds', 'emojiCdnUrl', 'imageMd5', 'videoMd5',
  'cardNickname', 'cardUsername', 'cardAvatarUrl', 'locationPoiname', 'locationLabel', 'finderNickname', 'finderCoverUrl',
  'finderDuration', 'chatRecordTitle',
].join(',');

export const PAGE_SIZE = 50;

/** A page of messages, newest first: the newest ones, or those older than a message's `_cursor`. */
export function getMessages(sessionId: string, before: string | null, signal?: AbortSignal): Promise<ParsedMessage[]> {
  return get<ParsedMessage[]>('api/chat/messages', {
    'session-id': sessionId, limit: PAGE_SIZE, before: before ?? undefined, parsed: true, fields: MESSAGE_FIELDS,
  }, signal);
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
