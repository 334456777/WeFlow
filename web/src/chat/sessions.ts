// What the conversation list shows (#197): its rows, the filters and folding; the kind of each session and its time
// are in sessionKind.ts.
import type { ChatSession, SessionStatus } from '../api/chat';
import { t } from '../i18n';
import { kindOf, type SessionFilter } from '../sessionKind';

/** The row that stands for the folded groups, as in WeChat. */
export const FOLDED_ID = '@folded';

/** A row of the conversation list. */
export interface SessionRow {
  id: string;
  name: string;
  avatarUrl?: string;
  /** The last message, with its sender in a group. */
  preview: string;
  time: number;
  unread: number;
  muted: boolean;
  /** Pinned in WeChat (#885): listed before the others. */
  pinned: boolean;
  /** The entry of the folded groups; `count` of them. */
  folded?: { count: number };
}

/** Where WeChat lists a conversation: pinning or unpinning it moves this to that moment (#894). */
const orderOf = (session: ChatSession) => session.sortTimestamp || session.lastTimestamp || 0;
/** The time shown on a row: that of the last message, which the preview shows (#894). */
const timeOf = (session: ChatSession) => session.lastTimestamp || session.sortTimestamp || 0;

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
    pinned: Boolean(status?.isPinned),
  };
}

function matches(session: ChatSession, query: string): boolean {
  return session.displayName.toLocaleLowerCase().includes(query) || session.username.toLocaleLowerCase().includes(query);
}

/** The conversations, then the official accounts not among them. */
export function withOfficials(sessions: readonly ChatSession[], officials: readonly ChatSession[]): ChatSession[] {
  const listed = new Set(sessions.map(s => s.username));
  return [...sessions, ...officials.filter(o => !listed.has(o.username))];
}

/**
 * The rows for a filter and a search. The conversations pinned in WeChat come first, newest first among them, as
 * WeChat lists them (#885). Folded groups are gathered into one entry at the time of the newest of them, or listed on
 * their own in the folded view; a search looks through all of them by name, in time order. The 「公众号」 filter adds
 * the `officials` followed that have no conversation, after the ones that do (#203).
 */
export function sessionRows(
  listed: readonly ChatSession[],
  statuses: Readonly<Record<string, SessionStatus>>,
  filter: SessionFilter,
  query: string,
  foldedView = false,
  officials: readonly ChatSession[] = [],
): SessionRow[] {
  const sessions = filter === 'official' && officials.length ? withOfficials(listed, officials) : listed;
  const wanted = (session: ChatSession) => filter === 'all' || kindOf(session.username) === filter;
  const isFolded = (session: ChatSession) => Boolean(statuses[session.username]?.isFolded);
  const search = query.trim().toLocaleLowerCase();
  if (search) return sessions.filter(s => wanted(s) && matches(s, search)).map(s => rowOf(s, statuses[s.username]));
  if (foldedView) return sessions.filter(isFolded).map(s => rowOf(s, statuses[s.username]));

  const rows: SessionRow[] = [];
  const folded = sessions.filter(isFolded);
  let entry: SessionRow | null = null;
  let entryOrder = 0;
  if (folded.length && (filter === 'all' || filter === 'group')) {
    const newest = folded[0];
    entryOrder = orderOf(newest);
    entry = {
      // The group's name in front of its last message, without the sender: one prefix is enough on one line.
      id: FOLDED_ID, name: t('折叠的群聊'), preview: (newest.displayName || newest.username) + '：' + newest.summary.replace(/\s+/g, ' ').trim(),
      time: timeOf(newest), unread: folded.reduce((sum, s) => sum + (s.unreadCount || 0), 0), muted: true, pinned: false,
      folded: { count: folded.length },
    };
  }
  const isPinned = (session: ChatSession) => Boolean(statuses[session.username]?.isPinned);
  const pinned: SessionRow[] = [];
  for (const session of sessions) {
    if (isFolded(session) || !wanted(session)) continue;
    if (isPinned(session)) { pinned.push(rowOf(session, statuses[session.username])); continue; }
    if (entry && entryOrder >= orderOf(session)) { rows.push(entry); entry = null; }
    rows.push(rowOf(session, statuses[session.username]));
  }
  if (entry) rows.push(entry);
  return [...pinned, ...rows];
}
