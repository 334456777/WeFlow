// What a conversation is with, and the time a list of conversations shows for it: the chat window's list, the
// export picker and the insights read them alike (#197).
import { formatDate, msg, relativeDay } from './i18n';

/** What a conversation is with: a person, a group or an official account. */
export type SessionKind = 'private' | 'group' | 'official';
/** What the list is filtered to. */
export type SessionFilter = 'all' | SessionKind;

/** Each filter's name, marked with `msg()` and shown through `t()`. */
export const filterLabels: Record<SessionFilter, string> = { all: msg('全部'), private: msg('私聊'), group: msg('群聊'), official: msg('公众号') };
/** Each kind's name, shown through `t()`. */
export const kindLabels: Record<SessionKind, string> = { private: msg('私聊'), group: msg('群聊'), official: msg('公众号') };

/** The kind of a conversation by its username: `…@chatroom` a group, `gh_…` an official account. */
export function kindOf(username: string): SessionKind {
  if (username.endsWith('@chatroom')) return 'group';
  if (username.startsWith('gh_')) return 'official';
  return 'private';
}

const pad = (n: number) => String(n).padStart(2, '0');

/** A session's time as WeChat shows it: 14:12 today, 昨天, a weekday this week, then the date (in the page's language). */
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
