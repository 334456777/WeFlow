// 会话的对象，以及会话列表为它显示的时间：聊天窗口的会话列表、导出的会话选择和见解都按这里的规则（#197）。
import { formatDate, msg, relativeDay } from './i18n';

/** 会话的对象：个人、群或公众号。 */
export type SessionKind = 'private' | 'group' | 'official';
/** 会话列表的筛选条件。 */
export type SessionFilter = 'all' | SessionKind;

/** 每个筛选条件的名称，用 `msg()` 标出，显示时经过 `t()`。 */
export const filterLabels: Record<SessionFilter, string> = { all: msg('全部'), private: msg('私聊'), group: msg('群聊'), official: msg('公众号') };
/** 每种会话的名称，显示时经过 `t()`。 */
export const kindLabels: Record<SessionKind, string> = { private: msg('私聊'), group: msg('群聊'), official: msg('公众号') };

/** 按用户名判定会话的对象：`…@chatroom` 是群，`gh_…` 是公众号，其余是个人。 */
export function kindOf(username: string): SessionKind {
  if (username.endsWith('@chatroom')) return 'group';
  if (username.startsWith('gh_')) return 'official';
  return 'private';
}

const pad = (n: number) => String(n).padStart(2, '0');

/**
 * 会话按微信的方式显示的时间：今天的显示时刻（14:12），昨天的显示「昨天」，七天内的显示星期，更早的显示日期；
 * 都用页面的语言。
 */
export function formatSessionTime(seconds: number, now = new Date()): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  const date = new Date(seconds * 1000);
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(now) - day(date)) / 86_400_000);
  if (days <= 0) return pad(date.getHours()) + ':' + pad(date.getMinutes());
  if (days === 1) return relativeDay(-1);
  if (days < 7) return formatDate(date, { weekday: 'long' });
  if (date.getFullYear() === now.getFullYear()) return formatDate(date, { month: 'numeric', day: 'numeric' });
  return formatDate(date, { year: 'numeric', month: 'numeric', day: 'numeric' });
}
