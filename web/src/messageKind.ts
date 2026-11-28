// What a message is, by its type and the kind the server names: how the chat window draws it.
import { baseType, type ParsedMessage } from './api/messages';

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
