// Chat analytics of the private chats (#207): `analytics overall`, `rankings`, `time`, and the people left out of them
// (`analytics excluded`, `exclude-candidates`).
import { call, get } from './client';

/** `analytics overall`: every private chat but those left out. */
export interface Overall {
  totalMessages: number;
  textMessages: number;
  imageMessages: number;
  voiceMessages: number;
  videoMessages: number;
  emojiMessages: number;
  otherMessages: number;
  sentMessages: number;
  receivedMessages: number;
  /** Unix seconds; null without messages. */
  firstMessageTime: number | null;
  lastMessageTime: number | null;
  /** Only an estimate, 20 for each month with messages, as the desktop app has it; not shown. */
  activeDays: number;
}

export interface Ranked {
  username: string;
  displayName: string;
  avatarUrl?: string;
  messageCount: number;
  sentCount: number;
  receivedCount: number;
  /** Unix seconds. */
  lastMessageTime: number;
}

/** `analytics time`: the keys are hours 0–23, weekdays 1 (Monday) – 7 (Sunday) and months `YYYY-MM`. */
export interface TimeSpread {
  hourlyDistribution: Record<string, number>;
  weekdayDistribution: Record<string, number>;
  monthlyDistribution: Record<string, number>;
}

export interface Candidate { username: string; displayName: string; avatarUrl?: string; wechatId?: string }

/** With `force`, read every chat again rather than the aggregate kept from last time. */
export const getOverall = (force: boolean, signal?: AbortSignal) => get<Overall>('analytics/overall', { force: force || undefined }, signal);
export const getRankings = (limit: number, signal?: AbortSignal) => get<Ranked[]>('analytics/rankings', { limit }, signal);
export const getTimeSpread = (signal?: AbortSignal) => get<TimeSpread>('analytics/time', {}, signal);
export const getExcluded = (signal?: AbortSignal) => get<string[]>('analytics/excluded', {}, signal);
export const getExcludeCandidates = (signal?: AbortSignal) => get<Candidate[]>('analytics/exclude-candidates', {}, signal);
/** Replaces the people left out; an empty `--set` clears them. Answers the list as saved. */
export const setExcluded = (usernames: string[]) => call<string[]>('PUT', 'analytics/excluded', { set: usernames.length ? usernames : '' });

export interface Bar { label: string; value: number }

/** The hours 0–23, none left out. */
export const hourBars = (spread: TimeSpread): Bar[] =>
  Array.from({ length: 24 }, (_, h) => ({ label: h + ' 点', value: spread.hourlyDistribution[h] ?? 0 }));

const WEEKDAYS = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];
export const weekdayBars = (spread: TimeSpread): Bar[] =>
  WEEKDAYS.map((label, i) => ({ label, value: spread.weekdayDistribution[i + 1] ?? 0 }));

/** Every month from the first with messages to the last, the quiet ones too. */
export function monthBars(spread: TimeSpread): Bar[] {
  const keys = Object.keys(spread.monthlyDistribution).filter(k => /^\d{4}-\d{2}$/.test(k)).sort();
  if (!keys.length) return [];
  const [y0, m0] = keys[0].split('-').map(Number);
  const [y1, m1] = keys[keys.length - 1].split('-').map(Number);
  const bars: Bar[] = [];
  for (let y = y0, m = m0; y < y1 || (y === y1 && m <= m1); m === 12 ? (y++, m = 1) : m++) {
    const key = y + '-' + String(m).padStart(2, '0');
    bars.push({ label: y + ' 年 ' + m + ' 月', value: spread.monthlyDistribution[key] ?? 0 });
  }
  return bars;
}

/** The kinds of messages, the most first, those with none left out. */
export function typeBars(o: Overall): Bar[] {
  return [
    { label: '文字', value: o.textMessages }, { label: '图片', value: o.imageMessages }, { label: '语音', value: o.voiceMessages },
    { label: '视频', value: o.videoMessages }, { label: '表情', value: o.emojiMessages }, { label: '其他', value: o.otherMessages },
  ].filter(b => b.value > 0).sort((a, b) => b.value - a.value);
}
