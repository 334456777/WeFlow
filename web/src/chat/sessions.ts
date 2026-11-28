// 会话列表显示的内容（#197）：列表的行、筛选和折叠；会话的种类和时间的写法在 sessionKind.ts。
import type { ChatSession, SessionStatus } from '../api/chat';
import { t } from '../i18n';
import { kindOf, type SessionFilter } from '../sessionKind';

/** 代表全部折叠的群聊的那一行的 ID。 */
export const FOLDED_ID = '@folded';

/** 会话列表的一行。 */
export interface SessionRow {
  id: string;
  name: string;
  avatarUrl?: string;
  /** 最后一条消息；群聊里带上发送者。 */
  preview: string;
  time: number;
  unread: number;
  muted: boolean;
  /** 在微信里置顶（#885）：排在其他会话前面。 */
  pinned: boolean;
  /** 折叠的群聊的入口，共 `count` 个群。 */
  folded?: { count: number };
}

/** 微信排列会话用的时间：置顶或取消置顶时变成操作的时刻（#894）。 */
const orderOf = (session: ChatSession) => session.sortTimestamp || session.lastTimestamp || 0;
/** 行上显示的时间：最后一条消息的时间，与预览显示的消息一致（#894）。 */
const timeOf = (session: ChatSession) => session.lastTimestamp || session.sortTimestamp || 0;

/** 最后一条消息的预览：合成一行；群聊里在前面加上发送者，自己发的不加。 */
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

/** 会话，后面接着不在其中的公众号。 */
export function withOfficials(sessions: readonly ChatSession[], officials: readonly ChatSession[]): ChatSession[] {
  const listed = new Set(sessions.map(s => s.username));
  return [...sessions, ...officials.filter(o => !listed.has(o.username))];
}

/**
 * 按筛选和搜索得出列表的行。在微信里置顶的会话排在最前，它们之间也是最新的在前（#885）。折叠的群聊合成一个入口，
 * 排在其中最新的群的位置；在折叠视图里则逐个列出。搜索按名称或用户名查找全部会话（包括折叠的），按时间排列。
 * 「公众号」筛选还在有会话的公众号后面加上关注了但没有会话的 `officials`（#203）。
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
      // 在最后一条消息前面加群名，不加发送者：一行里有一个前缀就够了。
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
