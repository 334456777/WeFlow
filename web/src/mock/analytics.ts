// The sample's chat analytics: three years of private chats with the people of WeFlow, counted as `analytics` counts
// them, and the groups' statistics counted from the sample's own group messages.
import type { ChatMessage, ContactRank, ExcludeCandidate, GroupChat, GroupRank, MediaCount, OverallStatistics, TimeDistribution } from '../api/contract.gen';
import { ago, dayEnd, dayStart, NOW } from './clock';
import { threads, type Message } from './chats';
import { groups, ME, people, person } from './people';

/** Messages with each friend over the years: sent by me, received. */
const PRIVATE: [username: string, sent: number, received: number][] = [
  ['wxid_sample_linmu', 4_620, 5_200], ['wxid_sample_chenyu', 3_980, 4_432], ['wxid_sample_zhouzhou', 2_910, 3_393],
  ['wxid_sample_xuzhiyuan', 2_240, 2_630], ['wxid_sample_hexi', 1_506, 1_750], ['wxid_sample_gunan', 1_420, 1_560],
  ['wxid_sample_songxiaoman', 780, 864], ['wxid_sample_shenxinghe', 188, 214], ['wxid_sample_chenyiwei', 130, 158],
];

const lastWith = (username: string): number | null => {
  const messages = threads.get(username);
  return messages?.length ? messages[messages.length - 1].createTime : null;
};

export function overall(): OverallStatistics {
  const sent = PRIVATE.reduce((n, [, s]) => n + s, 0);
  const received = PRIVATE.reduce((n, [, , r]) => n + r, 0);
  const total = sent + received;
  const image = Math.round(total * 0.087);
  const voice = Math.round(total * 0.032);
  const video = Math.round(total * 0.007);
  const emoji = Math.round(total * 0.041);
  const other = Math.round(total * 0.009);
  const text = total - image - voice - video - emoji - other;
  return {
    totalMessages: total, textMessages: text, imageMessages: image, voiceMessages: voice, videoMessages: video, emojiMessages: emoji,
    otherMessages: other, sentMessages: sent, receivedMessages: received, firstMessageTime: ago(3 * 365 + 40, 4), lastMessageTime: NOW - 600,
    activeDays: 941, messageTypeCounts: { 1: text, 3: image, 34: voice, 43: video, 47: emoji, 49: other },
  };
}

export const rankings = (limit: number): ContactRank[] => PRIVATE.slice(0, limit).map(([username, sent, received]) => ({
  username, displayName: person(username)?.name ?? username, wechatId: person(username)?.alias ?? username,
  messageCount: sent + received, sentCount: sent, receivedCount: received, lastMessageTime: lastWith(username),
}));

/** When the messages were sent: evenings and late nights, the way people working on a project after work chat. */
export function timeSpread(): TimeDistribution {
  const total = overall().totalMessages;
  const hourWeights = [5, 3, 1.5, 0.6, 0.3, 0.3, 0.6, 1.4, 2.6, 3.6, 4.4, 4.2, 3.4, 3.6, 4, 4.1, 4, 3.8, 3.6, 4.6, 6.2, 7.4, 7.8, 6.6];
  const weekWeights = [13, 13.5, 14, 13.2, 12.8, 17, 16.5];
  const share = (weights: number[]) => { const sum = weights.reduce((a, b) => a + b, 0); return weights.map(w => Math.round(total * w / sum)); };
  const hourly = Object.fromEntries(share(hourWeights).map((n, h) => [String(h), n]));
  const weekday = Object.fromEntries(share(weekWeights).map((n, i) => [String(i + 1), n]));
  // the last two years month by month, busier as the project grew
  const now = new Date();
  const months: [string, number][] = [];
  for (let i = 23; i >= 0; i--) {
    const date = new Date(now.getFullYear(), now.getMonth() - i, 1);
    const key = date.getFullYear() + '-' + String(date.getMonth() + 1).padStart(2, '0');
    months.push([key, Math.round(900 + (23 - i) * 95 + ((i * 37) % 11) * 60)]);
  }
  return { hourlyDistribution: hourly, weekdayDistribution: weekday, monthlyDistribution: Object.fromEntries(months) };
}

export const excludeCandidates = (): ExcludeCandidate[] => people.filter(p => p.username !== ME && !p.formerFriend)
  .map(p => ({ username: p.username, displayName: p.name, wechatId: p.alias ?? p.username }));

export const groupList = (): GroupChat[] => groups.map(g => ({ username: g.username, displayName: g.name, memberCount: g.members.length }));

/** A group's messages between two days (`YYYY-MM-DD`, either may be empty). */
export function groupMessages(chatroomId: string, start: string, end: string): Message[] {
  return (threads.get(chatroomId) ?? []).filter(m => m.senderUsername
    && (!start || m.createTime >= dayStart(start)) && (!end || m.createTime <= dayEnd(end)));
}

export function groupRanking(chatroomId: string, start: string, end: string, limit: number): GroupRank[] {
  const counts = new Map<string, number>();
  for (const m of groupMessages(chatroomId, start, end)) counts.set(m.senderUsername!, (counts.get(m.senderUsername!) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1]).slice(0, limit)
    .map(([username, messageCount]) => ({ member: { username, displayName: person(username)?.name ?? username }, messageCount }));
}

export function groupHours(chatroomId: string, start: string, end: string) {
  const hourly: Record<string, number> = {};
  for (const m of groupMessages(chatroomId, start, end)) {
    const hour = String(new Date(m.createTime * 1000).getHours());
    hourly[hour] = (hourly[hour] ?? 0) + 1;
  }
  return { hourlyDistribution: hourly };
}

const TYPE_NAMES: Record<number, string> = { 1: '文本', 3: '图片', 34: '语音', 42: '名片', 43: '视频', 47: '表情', 48: '位置', 49: '链接/文件', 50: '通话', 10000: '系统消息' };

export function groupMedia(chatroomId: string, start: string, end: string) {
  const counts = new Map<number, number>();
  const messages = groupMessages(chatroomId, start, end);
  for (const m of messages) counts.set(m.localType % 4_294_967_296, (counts.get(m.localType % 4_294_967_296) ?? 0) + 1);
  const typeCounts: MediaCount[] = [...counts].sort((a, b) => b[1] - a[1]).map(([type, count]) => ({ type, name: TYPE_NAMES[type] ?? String(type), count }));
  return { typeCounts, total: messages.length };
}

/** A member's messages in a group, newest first, 50 a page from `cursor` (how many were shown). */
export function memberMessages(chatroomId: string, username: string, start: string, end: string, cursor: number, limit: number) {
  const all = groupMessages(chatroomId, start, end).filter(m => m.senderUsername === username).reverse();
  const messages: ChatMessage[] = all.slice(cursor, cursor + limit);
  return { messages, hasMore: cursor + limit < all.length, nextCursor: cursor + messages.length };
}
