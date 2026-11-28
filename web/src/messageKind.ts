// 消息的种类，按消息类型和服务端给出的种类判定：聊天窗口按它画消息，双人报告按它称呼不是文字的消息（#1263）。
import { baseType, type ParsedMessage } from './api/messages';

/** 聊天窗口画一条消息的方式。 */
export type MessageKind =
  | 'text' | 'quote' | 'image' | 'emoji' | 'voice' | 'video' | 'file' | 'link' | 'finder' | 'record'
  | 'contact' | 'location' | 'call' | 'money' | 'system';

const APP_KINDS: Record<string, MessageKind> = {
  quote: 'quote', file: 'file', link: 'link', 'official-link': 'link', music: 'link', miniapp: 'link', announcement: 'text',
  solitaire: 'text', card: 'link', finder: 'finder', 'chat-record': 'record', 'red-packet': 'money', transfer: 'money',
  gift: 'money', location: 'location',
};

/** 拍一拍：子类型为 62 的 app 消息，导出时同样当作系统消息（#1200）。 */
const PAT = 62 * 4_294_967_296 + 49;

/** 消息的种类：按消息类型判定，app 消息再按 `appMsgKind` 判定；不认识的当作文字。 */
export function kindOf(message: Pick<ParsedMessage, 'localType' | 'appMsgKind'>): MessageKind {
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
