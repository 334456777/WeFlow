// The annual report (#208): `report annual years` and `report annual generate`, over one year or all of them.
import { call, get } from './client';
import { msg, t } from '../i18n';
import type * as Contract from './contract.gen';

/** `report annual generate`, as the contract says (#430). */
export type AnnualReport = Contract.AnnualReport;

export const getReportYears = (signal?: AbortSignal) => get<Contract.ReportYears>('api/report/annual/years', {}, signal);
// a report is generated, which the command API takes as a POST (#336)
export const getAnnualReport = (year: number, signal?: AbortSignal) => call<AnnualReport>('POST', 'report/annual/generate', { year }, signal);

/** A wait in seconds, as people say it. */
export function formatWait(seconds: number): string {
  if (seconds < 60) return t('{n} 秒', { n: Math.max(0, Math.round(seconds)) });
  if (seconds < 3600) return t('{n} 分钟', { n: Math.round(seconds / 60) });
  const hours = seconds / 3600;
  return t('{n} 小时', { n: hours < 10 ? Math.round(hours * 10) / 10 : Math.round(hours) });
}

/** Where the busiest hour of the week is: 「周六 21 点」, or null without messages. */
export function busiestHour(data: number[][]): { weekday: number; hour: number; count: number } | null {
  let best: { weekday: number; hour: number; count: number } | null = null;
  data.forEach((row, weekday) => row.forEach((count, hour) => { if (count > 0 && (!best || count > best.count)) best = { weekday, hour, count }; }));
  return best;
}

/** A message of the dual report; `createTime` is in milliseconds, `content` raw for anything but text. */
export type DualMessage = Contract.DualMessage;

/** `report dual generate`: me and one friend, over one year or all of them. */
export type DualReport = Contract.DualReport;

export const getDualReport = (friend: string, year: number, signal?: AbortSignal) => call<DualReport>('POST', 'report/dual/generate', { friend, year }, signal);

// shown through t()
const KINDS: Record<number, string> = {
  3: msg('[图片]'), 34: msg('[语音]'), 43: msg('[视频]'), 47: msg('[表情]'), 48: msg('[位置]'), 49: msg('[链接或文件]'), 50: msg('[通话]'),
  244813135921: msg('[引用回复]'),
};

/** What a message of the dual report says: its text, or its kind for anything else. */
export const messageText = (m: DualMessage) => (!m.localType || m.localType === 1 ? m.content : KINDS[m.localType] ? t(KINDS[m.localType]) : t('[消息]'));
