import { describe, expect, it } from 'vitest';
import type { ChatSession } from '../api/chat';
import { FOLDED_ID, formatSessionTime, kindOf, previewOf, sessionRows } from './sessions';

const at = (iso: string) => Math.floor(new Date(iso).getTime() / 1000);

function session(username: string, displayName: string, time: number, extra: Partial<ChatSession> = {}): ChatSession {
  return { username, displayName, summary: '消息', unreadCount: 0, sortTimestamp: time, lastTimestamp: time, lastMsgType: 1, ...extra };
}

const sessions = [
  session('wxid_a', '林沐', 500),
  session('g1@chatroom', '家人群', 400, { unreadCount: 3 }),
  session('gh_news', '早报', 300),
  session('g2@chatroom', '旧同学', 200, { unreadCount: 2 }),
  session('wxid_b', '陈屿', 100),
];

describe('kindOf', () => {
  it('tells private chats, groups and official accounts apart', () => {
    expect(kindOf('wxid_a')).toBe('private');
    expect(kindOf('123@chatroom')).toBe('group');
    expect(kindOf('gh_abc')).toBe('official');
  });
});

describe('previewOf', () => {
  it('puts the sender of a group message in front, unless it was me, and keeps one line', () => {
    expect(previewOf(session('g@chatroom', '群', 1, { summary: '好的\n明天见', lastSenderDisplayName: '李婷', lastMsgSender: 'wxid_l', selfWxid: 'wxid_me' }))).toBe('李婷：好的 明天见');
    expect(previewOf(session('g@chatroom', '群', 1, { summary: '收到', lastSenderDisplayName: '我', lastMsgSender: 'wxid_me', selfWxid: 'wxid_me' }))).toBe('收到');
    expect(previewOf(session('wxid_a', '林沐', 1, { summary: '你好', lastSenderDisplayName: '林沐' }))).toBe('你好');
  });
});

describe('sessionRows', () => {
  const statuses = { 'g1@chatroom': { isFolded: true, isMuted: true }, 'g2@chatroom': { isFolded: true, isMuted: false }, wxid_b: { isFolded: false, isMuted: true } };

  it('gathers the folded groups into one entry at the time of the newest', () => {
    const rows = sessionRows(sessions, statuses, 'all', '');
    expect(rows.map(r => r.id)).toEqual(['wxid_a', FOLDED_ID, 'gh_news', 'wxid_b']);
    expect(rows[1]).toMatchObject({ name: '折叠的群聊', time: 400, unread: 5, folded: { count: 2 }, preview: '家人群：消息' });
    expect(rows[3].muted).toBe(true);
  });

  it('filters by kind, lists the folded groups on their own, and searches all of them by name', () => {
    expect(sessionRows(sessions, statuses, 'private', '').map(r => r.id)).toEqual(['wxid_a', 'wxid_b']);
    expect(sessionRows(sessions, statuses, 'group', '').map(r => r.id)).toEqual([FOLDED_ID]);
    expect(sessionRows(sessions, statuses, 'official', '').map(r => r.id)).toEqual(['gh_news']);
    expect(sessionRows(sessions, statuses, 'all', '', true).map(r => r.id)).toEqual(['g1@chatroom', 'g2@chatroom']);
    expect(sessionRows(sessions, statuses, 'all', ' 同学 ').map(r => r.id)).toEqual(['g2@chatroom']);
    expect(sessionRows(sessions, statuses, 'private', 'WXID').map(r => r.id)).toEqual(['wxid_a', 'wxid_b']);
  });

  it('shows everything when the states are not known yet', () => {
    expect(sessionRows(sessions, {}, 'group', '').map(r => r.id)).toEqual(['g1@chatroom', 'g2@chatroom']);
  });
});

describe('formatSessionTime', () => {
  const now = new Date('2026-10-16T10:00:00');
  it('shows the time today, then 昨天, a weekday, the date and the year', () => {
    expect(formatSessionTime(at('2026-10-16T08:05:00'), now)).toBe('08:05');
    expect(formatSessionTime(at('2026-10-15T23:59:00'), now)).toBe('昨天');
    expect(formatSessionTime(at('2026-10-12T12:00:00'), now)).toBe('星期一');
    expect(formatSessionTime(at('2026-10-09T12:00:00'), now)).toBe('10/9');
    expect(formatSessionTime(at('2025-12-31T12:00:00'), now)).toBe('2025/12/31');
    expect(formatSessionTime(0, now)).toBe('');
  });
});
