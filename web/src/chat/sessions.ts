// What the conversation list shows (#197): the kind of each session, the filters, folding and the times.
import type { ChatSession, SessionStatus } from '../api/chat';

export type SessionKind = 'private' | 'group' | 'official';
export type SessionFilter = 'all' | SessionKind;

export const filterLabels: Record<SessionFilter, string> = { all: '全部', private: '私聊', group: '群聊', official: '公众号' };
export const kindLabels: Record<SessionKind, string> = { private: '私聊', group: '群聊', official: '公众号' };

/** The row that stands for the folded groups, as in WeChat. */
export const FOLDED_ID = '@folded';

export function kindOf(username: string): SessionKind {
  if (username.endsWith('@chatroom')) return 'group';
  if (username.startsWith('gh_')) return 'official';
  return 'private';
}

export interface SessionRow {
  id: string;
  name: string;
  avatarUrl?: string;
  /** The last message, with its sender in a group. */
  preview: string;
  time: number;
  unread: number;
  muted: boolean;
  /** The entry of the folded groups; `count` of them. */
  folded?: { count: number };
}

const timeOf = (session: ChatSession) => session.sortTimestamp || session.lastTimestamp || 0;

/** The last message as WeChat shows it: one line, and in a group whose sender it was, unless it was me. */
export function previewOf(session: ChatSession): string {
  const text = session.summary.replace(/\s+/g, ' ').trim();
  const sender = session.lastSenderDisplayName?.trim();
  const mine = session.lastMsgSender && session.lastMsgSender === session.selfWxid;
  return kindOf(session.username) === 'group' && sender && !mine ? sender + '：' + text : text;
}

function rowOf(session: ChatSession, status?: SessionStatus): SessionRow {
  return {
    id: session.username,
    name: session.displayName || session.username,
    avatarUrl: session.avatarUrl || undefined,
    preview: previewOf(session),
    time: timeOf(session),
    unread: session.unreadCount || 0,
    muted: Boolean(status?.isMuted),
  };
}

function matches(session: ChatSession, query: string): boolean {
  return session.displayName.toLocaleLowerCase().includes(query) || session.username.toLocaleLowerCase().includes(query);
}

/**
 * The rows for a filter and a search. Folded groups are gathered into one entry at the time of the newest of them,
 * or listed on their own in the folded view; a search looks through all of them by name.
 */
export function sessionRows(
  sessions: readonly ChatSession[],
  statuses: Readonly<Record<string, SessionStatus>>,
  filter: SessionFilter,
  query: string,
  foldedView = false,
): SessionRow[] {
  const wanted = (session: ChatSession) => filter === 'all' || kindOf(session.username) === filter;
  const isFolded = (session: ChatSession) => Boolean(statuses[session.username]?.isFolded);
  const search = query.trim().toLocaleLowerCase();
  if (search) return sessions.filter(s => wanted(s) && matches(s, search)).map(s => rowOf(s, statuses[s.username]));
  if (foldedView) return sessions.filter(isFolded).map(s => rowOf(s, statuses[s.username]));

  const rows: SessionRow[] = [];
  const folded = sessions.filter(isFolded);
  let entry: SessionRow | null = null;
  if (folded.length && (filter === 'all' || filter === 'group')) {
    const newest = folded[0];
    entry = {
      // The group's name in front of its last message, without the sender: one prefix is enough on one line.
      id: FOLDED_ID, name: '折叠的群聊', preview: (newest.displayName || newest.username) + '：' + newest.summary.replace(/\s+/g, ' ').trim(),
      time: timeOf(newest), unread: folded.reduce((sum, s) => sum + (s.unreadCount || 0), 0), muted: true,
      folded: { count: folded.length },
    };
  }
  for (const session of sessions) {
    if (isFolded(session) || !wanted(session)) continue;
    if (entry && entry.time >= timeOf(session)) { rows.push(entry); entry = null; }
    rows.push(rowOf(session, statuses[session.username]));
  }
  if (entry) rows.push(entry);
  return rows;
}

const WEEKDAYS = ['星期日', '星期一', '星期二', '星期三', '星期四', '星期五', '星期六'];
const pad = (n: number) => String(n).padStart(2, '0');

/** A session's time as WeChat shows it: 14:12 today, 昨天, a weekday this week, then the date. */
export function formatSessionTime(seconds: number, now = new Date()): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  const date = new Date(seconds * 1000);
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(now) - day(date)) / 86_400_000);
  if (days <= 0) return pad(date.getHours()) + ':' + pad(date.getMinutes());
  if (days === 1) return '昨天';
  if (days < 7) return WEEKDAYS[date.getDay()];
  if (date.getFullYear() === now.getFullYear()) return (date.getMonth() + 1) + '/' + date.getDate();
  return date.getFullYear() + '/' + (date.getMonth() + 1) + '/' + date.getDate();
}
