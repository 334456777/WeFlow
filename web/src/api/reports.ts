// The annual report (#208): `report annual years` and `report annual generate`, over one year or all of them.
import { get } from './client';

export interface Person { username?: string; displayName: string; avatarUrl?: string }

export interface AnnualReport {
  /** 0 for all the years. */
  year: number;
  totalMessages: number;
  totalFriends: number;
  /** The three private chats with the most messages. */
  coreFriends: (Person & { username: string; messageCount: number; sentCount: number; receivedCount: number })[];
  /** Months 1–12; `displayName` is 「暂无」 for a month without messages. */
  monthlyTopFriends: (Person & { month: number; messageCount: number })[];
  peakDay: { date: string; messageCount: number; topFriend: string; topFriendCount: number } | null;
  longestStreak: { friendName: string; days: number; startDate: string; endDate: string } | null;
  /** Messages by weekday (Monday first) and hour. */
  activityHeatmap: { data: number[][] };
  /** Who the most messages between 0:00 and 6:00 are with; `percentage` of all those. */
  midnightKing: { displayName: string; count: number; percentage: number } | null;
  /** The friend whose sent and received messages, 50 or more each, are closest to even; `ratio` is sent ÷ received. */
  mutualFriend: (Person & { sentCount: number; receivedCount: number; ratio: number }) | null;
  /** Conversations opened by me or by them; `initiativeRate` in percent. */
  socialInitiative: { initiatedChats: number; receivedChats: number; initiativeRate: number; topInitiatedFriend?: string; topInitiatedCount?: number } | null;
  /** Seconds. */
  responseSpeed: { avgResponseTime: number; fastestFriend: string; fastestTime: number } | null;
  topPhrases: { phrase: string; count: number }[];
  snsStats?: { totalPosts: number; topLikers: (Person & { count: number })[]; topLiked: (Person & { count: number })[] };
  /** A friend talked to much more in an earlier period (`periodDesc`) than later. */
  lostFriend: (Person & { earlyCount: number; lateCount: number; periodDesc: string }) | null;
}

export const getReportYears = (signal?: AbortSignal) => get<{ years: number[] }>('report/annual/years', {}, signal);
export const getAnnualReport = (year: number, signal?: AbortSignal) => get<AnnualReport>('report/annual/generate', { year }, signal);

/** A wait in seconds, as people say it. */
export function formatWait(seconds: number): string {
  if (seconds < 60) return Math.max(0, Math.round(seconds)) + ' 秒';
  if (seconds < 3600) return Math.round(seconds / 60) + ' 分钟';
  const hours = seconds / 3600;
  return (hours < 10 ? Math.round(hours * 10) / 10 : Math.round(hours)) + ' 小时';
}

/** Where the busiest hour of the week is: 「周六 21 点」, or null without messages. */
export function busiestHour(data: number[][]): { weekday: number; hour: number; count: number } | null {
  let best: { weekday: number; hour: number; count: number } | null = null;
  data.forEach((row, weekday) => row.forEach((count, hour) => { if (count > 0 && (!best || count > best.count)) best = { weekday, hour, count }; }));
  return best;
}

/** A message of the dual report; `createTime` is in milliseconds, `content` raw for anything but text. */
export interface DualMessage { content: string; isSentByMe: boolean; createTime: number; localType?: number }

/** `report dual generate`: me and one friend, over one year or all of them. */
export interface DualReport {
  year: number;
  selfName: string;
  selfAvatarUrl?: string;
  friendUsername: string;
  friendName: string;
  friendAvatarUrl?: string;
  firstChat: DualMessage | null;
  /** The first messages ever between the two. */
  firstChatMessages?: DualMessage[];
  /** The first messages of the year. */
  yearFirstChat?: (DualMessage & { firstThreeMessages: DualMessage[] }) | null;
  stats: { totalMessages: number; totalWords: number; imageCount: number; voiceCount: number; emojiCount: number };
  topPhrases: { phrase: string; count: number }[];
  myExclusivePhrases: { phrase: string; count: number }[];
  friendExclusivePhrases: { phrase: string; count: number }[];
  /** Weekday (Monday first) × hour. */
  heatmap?: number[][];
  /** Conversations opened by me, or by them. */
  initiative?: { initiated: number; received: number };
  /** Seconds. */
  response?: { avg: number; fastest: number; slowest?: number; count: number };
  /** `YYYY-MM` → messages. */
  monthly?: Record<string, number>;
  streak?: { days: number; startDate: string; endDate: string };
}

export const getDualReport = (friend: string, year: number, signal?: AbortSignal) => get<DualReport>('report/dual/generate', { friend, year }, signal);

const KINDS: Record<number, string> = { 3: '[图片]', 34: '[语音]', 43: '[视频]', 47: '[表情]', 48: '[位置]', 49: '[链接或文件]', 50: '[通话]', 244813135921: '[引用回复]' };

/** What a message of the dual report says: its text, or its kind for anything else. */
export const messageText = (m: DualMessage) => (!m.localType || m.localType === 1 ? m.content : KINDS[m.localType] ?? '[消息]');
