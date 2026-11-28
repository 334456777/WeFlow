// How the chat window shows a parsed message (#198): its labels, and where the date changes; its kind is in
// messageKind.ts.
import type { ParsedMessage } from '../api/messages';
import { t } from '../i18n';

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

/** A voice's length as WeChat writes it: `12″`, `1′05″`. */
export function formatDuration(seconds: number | undefined): string {
  const s = Math.max(0, Math.round(seconds ?? 0));
  return s < 60 ? s + '″' : Math.floor(s / 60) + '′' + String(s % 60).padStart(2, '0') + '″';
}
