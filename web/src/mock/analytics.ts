// 示例数据的聊天分析：与 WeFlow 的朋友和账号本人生活中的人三年来的私聊，按 `analytics` 的方式统计；
// 群的统计由示例数据自己的群消息算出。
import type { ChatMessage, ContactRank, ExcludeCandidate, GroupChat, GroupRank, MediaCount, OverallStatistics, TimeDistribution } from '../api/contract.gen';
import { ago, dayEnd, dayStart, NOW } from './clock';
import { threads, type Message } from './chats';
import { groups, ME, people, person } from './people';

/** 这些年和每位朋友的消息数：我发的、收到的。 */
const PRIVATE: [username: string, sent: number, received: number][] = [
  // 聊天里说排第一的那位，消息数与聊天里说的三万多条一致
  ['wxid_sample_xiaolu', 15_820, 16_596], ['wxid_sample_linmu', 4_620, 5_200], ['wxid_sample_chenyu', 3_980, 4_432],
  ['wxid_sample_mom', 2_960, 3_870], ['wxid_sample_zhouzhou', 2_910, 3_393], ['wxid_sample_xuzhiyuan', 2_240, 2_630],
  ['wxid_sample_ajie', 1_680, 1_720], ['wxid_sample_hexi', 1_506, 1_750], ['wxid_sample_gunan', 1_420, 1_560],
  ['wxid_sample_teacherchen', 402, 610], ['wxid_sample_songxiaoman', 780, 864], ['wxid_sample_youzi', 320, 388],
  ['wxid_sample_shenxinghe', 188, 214], ['wxid_sample_chenyiwei', 130, 158],
];

const lastWith = (username: string): number | null => {
  const messages = threads.get(username);
  return messages?.length ? messages[messages.length - 1].createTime : null;
};

/** 不参与统计的人（`analytics excluded`），页面打开期间保存在页面里（#896）。 */
let excluded: string[] = [];

/** `analytics excluded`：当前不参与统计的人。 */
export const excludedList = (): string[] => [...excluded];

/** `analytics excluded --set`：替换列表，为空时清空；回答保存后的列表。 */
export function setExcluded(usernames: readonly string[]): string[] {
  excluded = [...new Set(usernames.filter(Boolean))];
  return excludedList();
}

/** 参与统计的私聊：没有被排除的人的私聊。 */
const counted = () => PRIVATE.filter(([username]) => !excluded.includes(username));

/** 参与统计的私聊的 `analytics overall`。 */
export function overall(): OverallStatistics {
  const sent = counted().reduce((n, [, s]) => n + s, 0);
  const received = counted().reduce((n, [, , r]) => n + r, 0);
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

/** `analytics rankings`：参与统计的前 `limit` 个私聊，为 0 时全部列出。 */
export const rankings = (limit: number): ContactRank[] => counted().slice(0, limit || undefined).map(([username, sent, received]) => ({
  username, displayName: person(username)?.name ?? username, wechatId: person(username)?.alias ?? username,
  messageCount: sent + received, sentCount: sent, receivedCount: received, lastMessageTime: lastWith(username),
}));

/** 消息的发送时间：集中在晚上和深夜，就像下班后一起做项目的人聊天那样。 */
export function timeSpread(): TimeDistribution {
  const total = overall().totalMessages;
  const hourWeights = [5, 3, 1.5, 0.6, 0.3, 0.3, 0.6, 1.4, 2.6, 3.6, 4.4, 4.2, 3.4, 3.6, 4, 4.1, 4, 3.8, 3.6, 4.6, 6.2, 7.4, 7.8, 6.6];
  const weekWeights = [13, 13.5, 14, 13.2, 12.8, 17, 16.5];
  const share = (weights: number[]) => { const sum = weights.reduce((a, b) => a + b, 0); return weights.map(w => Math.round(total * w / sum)); };
  const hourly = Object.fromEntries(share(hourWeights).map((n, h) => [String(h), n]));
  const weekday = Object.fromEntries(share(weekWeights).map((n, i) => [String(i + 1), n]));
  // 最近两年按月统计，项目越做越大，消息也越来越多
  const now = new Date();
  const months: [string, number][] = [];
  for (let i = 23; i >= 0; i--) {
    const date = new Date(now.getFullYear(), now.getMonth() - i, 1);
    const key = date.getFullYear() + '-' + String(date.getMonth() + 1).padStart(2, '0');
    months.push([key, Math.round(900 + (23 - i) * 95 + ((i * 37) % 11) * 60)]);
  }
  return { hourlyDistribution: hourly, weekdayDistribution: weekday, monthlyDistribution: Object.fromEntries(months) };
}

/** `analytics exclude-candidates`：除账号本人和曾经的好友以外的所有人。 */
export const excludeCandidates = (): ExcludeCandidate[] => people.filter(p => p.username !== ME && !p.formerFriend)
  .map(p => ({ username: p.username, displayName: p.name, wechatId: p.alias ?? p.username }));

/** `group list`：示例数据里的群。 */
export const groupList = (): GroupChat[] => groups.map(g => ({ username: g.username, displayName: g.name, memberCount: g.members.length }));

/** 群在两个日期之间、有发送者的消息（日期写成 `YYYY-MM-DD`，都可以为空）。 */
export function groupMessages(chatroomId: string, start: string, end: string): Message[] {
  return (threads.get(chatroomId) ?? []).filter(m => m.senderUsername
    && (!start || m.createTime >= dayStart(start)) && (!end || m.createTime <= dayEnd(end)));
}

/** `group ranking`：两个日期之间在群里发消息最多的前 `limit` 人，为 0 时全部列出。 */
export function groupRanking(chatroomId: string, start: string, end: string, limit: number): GroupRank[] {
  const counts = new Map<string, number>();
  for (const m of groupMessages(chatroomId, start, end)) counts.set(m.senderUsername!, (counts.get(m.senderUsername!) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1]).slice(0, limit || undefined)
    .map(([username, messageCount]) => ({ member: { username, displayName: person(username)?.name ?? username }, messageCount }));
}

/** `group hours`：两个日期之间群消息按本地时间的小时分布。 */
export function groupHours(chatroomId: string, start: string, end: string) {
  const hourly: Record<string, number> = {};
  for (const m of groupMessages(chatroomId, start, end)) {
    const hour = String(new Date(m.createTime * 1000).getHours());
    hourly[hour] = (hourly[hour] ?? 0) + 1;
  }
  return { hourlyDistribution: hourly };
}

const TYPE_NAMES: Record<number, string> = { 1: '文本', 3: '图片', 34: '语音', 42: '名片', 43: '视频', 47: '表情', 48: '位置', 49: '链接/文件', 50: '通话', 10000: '系统消息' };

/** `group media`：两个日期之间群消息按类型的分布，最多的在前。 */
export function groupMedia(chatroomId: string, start: string, end: string) {
  const counts = new Map<number, number>();
  const messages = groupMessages(chatroomId, start, end);
  for (const m of messages) counts.set(m.localType % 4_294_967_296, (counts.get(m.localType % 4_294_967_296) ?? 0) + 1);
  const typeCounts: MediaCount[] = [...counts].sort((a, b) => b[1] - a[1]).map(([type, count]) => ({ type, name: TYPE_NAMES[type] ?? String(type), count }));
  return { typeCounts, total: messages.length };
}

/** 一个成员在群里的消息，最新的在前：从 `cursor`（已经显示的条数）起的一页，每页 `limit` 条。 */
export function memberMessages(chatroomId: string, username: string, start: string, end: string, cursor: number, limit: number) {
  const all = groupMessages(chatroomId, start, end).filter(m => m.senderUsername === username).reverse();
  const messages: ChatMessage[] = all.slice(cursor, cursor + limit);
  return { messages, hasMore: cursor + limit < all.length, nextCursor: cursor + messages.length };
}
