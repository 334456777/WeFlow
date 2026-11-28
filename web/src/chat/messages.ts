// 聊天窗口显示解析过的消息时用的函数（#198）：是不是自己发的、卡片底部的标签、去掉文字开头的标签。
// 消息的种类在 messageKind.ts，日期和时间的写法在 format.ts。
import type { ParsedMessage } from '../api/messages';
import { t } from '../i18n';

/** 消息是否是账号所有者自己发的。 */
export const isMine = (message: ParsedMessage) => message.isSend === 1;

/** app 消息卡片底部的标签：有来源（公众号、小程序、应用）的名称时用名称，否则用种类的名称。 */
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

/** 去掉开头的标签（如 `[文件] `）后的文字，卡片会用自己的方式表示这个标签。 */
export function withoutLabel(text: string): string {
  return text.replace(/^\[[^\]]{1,8}\]\s*/, '');
}
