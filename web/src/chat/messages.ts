// How the chat window shows a parsed message (#198): its kind, its labels, and where the date changes.
import { baseType, type ParsedMessage } from '../api/messages';
import { t } from '../i18n';

/** How the chat window draws a message. */
export type MessageKind =
  | 'text' | 'quote' | 'image' | 'emoji' | 'voice' | 'video' | 'file' | 'link' | 'finder' | 'record'
  | 'contact' | 'location' | 'call' | 'money' | 'system';

const APP_KINDS: Record<string, MessageKind> = {
  quote: 'quote', file: 'file', link: 'link', 'official-link': 'link', music: 'link', miniapp: 'link', announcement: 'text',
  solitaire: 'text', card: 'link', finder: 'finder', 'chat-record': 'record', 'red-packet': 'money', transfer: 'money',
  gift: 'money', location: 'location',
};

/** A pat (拍一拍): the app message of type 62, which the exports keep as a system message too (#1200). */
const PAT = 62 * 4_294_967_296 + 49;

/** A message's kind, by its type and, for an app message, by its `appMsgKind`; text for one it does not know. */
export function kindOf(message: ParsedMessage): MessageKind {
  if (message.localType === PAT) return 'system';
  switch (baseType(message.localType)) {
    case 1: return 'text';
    case 3: return 'image';
    case 34: return 'voice';
    case 42: return 'contact';
    case 43: return 'video';
    case 47: return 'emoji';
    case 48: return 'location';
    case 50: return 'call';
    case 10000: case 10002: return 'system';
    case 49: return APP_KINDS[message.appMsgKind ?? ''] ?? 'text';
    default: return 'text';
  }
}

/** Whether the account sent it. */
export const isMine = (message: ParsedMessage) => message.isSend === 1;

/** The text of an app message's kind, for the corner of its card. */
export function appLabel(message: ParsedMessage): string {
  switch (message.appMsgKind) {
    case 'official-link': return message.appMsgSourceName || t('公众号文章');
    case 'miniapp': return message.appMsgSourceName || message.appMsgAppName || t('小程序');
    case 'music': return t('音乐');
    case 'card': return message.appMsgAppName || t('卡片');
    case 'red-packet': return t('红包');
    case 'transfer': return t('转账');
    case 'gift': return t('礼物');
    default: return message.appMsgAppName || message.appMsgSourceName || t('链接');
  }
}

/** The text without a leading label such as 「[文件] 」, which the card says in its own way. */
export function withoutLabel(text: string): string {
  return text.replace(/^\[[^\]]{1,8}\]\s*/, '');
}
