// How the chat window shows a parsed message (#198): its kind, its labels, and where the date changes.
import type { ParsedMessage } from '../api/messages';

export type MessageKind =
  | 'text' | 'quote' | 'image' | 'emoji' | 'voice' | 'video' | 'file' | 'link' | 'finder' | 'record'
  | 'contact' | 'location' | 'call' | 'money' | 'system';

/** WeChat keeps an app message's kind in the high 32 bits of its type; the low bits are the type itself. */
export const baseType = (localType: number) => localType % 4_294_967_296;

const APP_KINDS: Record<string, MessageKind> = {
  quote: 'quote', file: 'file', link: 'link', 'official-link': 'link', music: 'link', miniapp: 'link', announcement: 'text',
  solitaire: 'text', card: 'link', finder: 'finder', 'chat-record': 'record', 'red-packet': 'money', transfer: 'money',
  gift: 'money', location: 'location',
};

export function kindOf(message: ParsedMessage): MessageKind {
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

export const isMine = (message: ParsedMessage) => message.isSend === 1;

/** The text of an app message's kind, for the corner of its card. */
export function appLabel(message: ParsedMessage): string {
  switch (message.appMsgKind) {
    case 'official-link': return message.appMsgSourceName || '公众号文章';
    case 'miniapp': return message.appMsgSourceName || message.appMsgAppName || '小程序';
    case 'music': return '音乐';
    case 'card': return message.appMsgAppName || '卡片';
    case 'red-packet': return '红包';
    case 'transfer': return '转账';
    case 'gift': return '礼物';
    default: return message.appMsgAppName || message.appMsgSourceName || '链接';
  }
}

/** The text without a leading label such as 「[文件] 」, which the card says in its own way. */
export function withoutLabel(text: string): string {
  return text.replace(/^\[[^\]]{1,8}\]\s*/, '');
}

export function formatFileSize(bytes: number | undefined): string {
  if (!bytes || bytes < 0) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return (unit === 0 ? String(value) : value.toFixed(value < 10 ? 1 : 0)) + ' ' + units[unit];
}

export function formatDuration(seconds: number | undefined): string {
  const s = Math.max(0, Math.round(seconds ?? 0));
  return s < 60 ? s + '″' : Math.floor(s / 60) + '′' + String(s % 60).padStart(2, '0') + '″';
}

const pad = (n: number) => String(n).padStart(2, '0');
const WEEKDAYS = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

export const formatClock = (seconds: number) => {
  const date = new Date(seconds * 1000);
  return pad(date.getHours()) + ':' + pad(date.getMinutes());
};

/** The local day of a message, as `2026-10-12`. */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/** The heading of a day: 今天, 昨天, or the date with its weekday (and the year when it is not this year). */
export function formatDayHeading(day: string, now = new Date()): string {
  const [y, m, d] = day.split('-').map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  if (days === 0) return '今天';
  if (days === 1) return '昨天';
  return (y === now.getFullYear() ? '' : y + '年') + m + '月' + d + '日 ' + WEEKDAYS[date.getDay()];
}
