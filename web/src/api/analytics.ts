// 私聊的聊天分析（#207）：`analytics overall`、`rankings`、`time`，以及不参与统计的人（`analytics excluded`、
// `exclude-candidates`）。
import { call, get } from './client';
import type * as Contract from './contract.gen';
import { formatDate, t } from '../i18n';

/** `analytics overall`：除去不参与统计的人以外的全部私聊（契约的类型，#430）。 */
export type Overall = Contract.OverallStatistics;
/** `analytics rankings` 的一个私聊。 */
export type Ranked = Contract.ContactRank;
/** `analytics time`：键是 0–23 时、星期 1（周一）到 7（周日）和月份 `YYYY-MM`。 */
export type TimeSpread = Contract.TimeDistribution;
/** `analytics exclude-candidates` 给出的可以排除的一个人。 */
export type Candidate = Contract.ExcludeCandidate;

/** `analytics overall`，在服务进程内读取（`/api/`）。 */
export const getOverall = (signal?: AbortSignal) => get<Overall>('api/analytics/overall', {}, signal);
/** `analytics rankings`：消息最多的 `limit` 个私聊。 */
export const getRankings = (limit: number, signal?: AbortSignal) => get<Ranked[]>('api/analytics/rankings', { limit }, signal);
/** `analytics time`：私聊消息按小时、星期和月份的分布。 */
export const getTimeSpread = (signal?: AbortSignal) => get<TimeSpread>('api/analytics/time', {}, signal);
/** `analytics excluded`：不参与统计的人的用户名。 */
export const getExcluded = (signal?: AbortSignal) => get<string[]>('api/analytics/excluded', {}, signal);
/** `analytics exclude-candidates`：可以排除的人。 */
export const getExcludeCandidates = (signal?: AbortSignal) => get<Candidate[]>('api/analytics/exclude-candidates', {}, signal);
/** `analytics excluded --set`：替换不参与统计的人，`--set` 为空时清空；回答保存后的列表。 */
export const setExcluded = (usernames: string[]) => call<string[]>('PUT', 'analytics/excluded', { set: usernames.length ? usernames : '' });

/** 图表里的一根柱子；表示月份的柱子还带有年份和月份。 */
export interface Bar { label: string; value: number; year?: number; month?: number }

/** 用网页的语言写出的月份简称：`11月`、`Nov`。 */
export const monthName = (month: number) => formatDate(new Date(2000, month - 1, 1), { month: 'short' });

/** 按小时的计数画成 0–23 时的柱子，每个小时都有，没有消息的也有。 */
export const hourBars = (hourly: Record<string, number>): Bar[] =>
  Array.from({ length: 24 }, (_, h) => ({ label: t('{h} 点', { h }), value: hourly[h] ?? 0 }));

/** 周一到周日的柱子，标签是网页语言的星期简称（`周一`、`Mon`）。 */
export const weekdayBars = (spread: TimeSpread): Bar[] =>
  Array.from({ length: 7 }, (_, i) => ({
    label: formatDate(new Date(2024, 0, 1 + i), { weekday: 'short' }), // 2024-01-01 是星期一
    value: spread.weekdayDistribution[i + 1] ?? 0,
  }));

/** 从第一个有消息的月份到最后一个的每个月，没有消息的月份也有柱子。 */
export function monthBars(spread: TimeSpread): Bar[] {
  const keys = Object.keys(spread.monthlyDistribution).filter(k => /^\d{4}-\d{2}$/.test(k)).sort();
  if (!keys.length) return [];
  const [y0, m0] = keys[0].split('-').map(Number);
  const [y1, m1] = keys[keys.length - 1].split('-').map(Number);
  const bars: Bar[] = [];
  for (let y = y0, m = m0; y < y1 || (y === y1 && m <= m1); m === 12 ? (y++, m = 1) : m++) {
    const key = y + '-' + String(m).padStart(2, '0');
    bars.push({ label: formatDate(new Date(y, m - 1, 1), { year: 'numeric', month: 'short' }), value: spread.monthlyDistribution[key] ?? 0, year: y, month: m });
  }
  return bars;
}

/** 各种消息的柱子，最多的在前，没有消息的种类不列：用于私聊的统计或一个群成员的统计。 */
export function typeBars(o: Pick<Overall, 'textMessages' | 'imageMessages' | 'voiceMessages' | 'videoMessages' | 'emojiMessages' | 'otherMessages'>): Bar[] {
  return [
    { label: t('文字'), value: o.textMessages }, { label: t('图片'), value: o.imageMessages }, { label: t('语音'), value: o.voiceMessages },
    { label: t('视频'), value: o.videoMessages }, { label: t('表情'), value: o.emojiMessages }, { label: t('其他'), value: o.otherMessages },
  ].filter(b => b.value > 0).sort((a, b) => b.value - a.value);
}
