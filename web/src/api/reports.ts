// The annual report (#208): `report annual years` and `report annual generate`, over one year or all of them.
import { call, get } from './client';
import { msg, t } from '../i18n';
import { kindOf, type MessageKind } from '../messageKind';
import type * as Contract from './contract.gen';

/** `report annual generate`, as the contract says (#430). */
export type AnnualReport = Contract.AnnualReport;

/** `report annual years`, read in the server's process. */
export const getReportYears = (signal?: AbortSignal) => get<Contract.ReportYears>('api/report/annual/years', {}, signal);
/** `report annual generate` over `year` (0 for all the years), a POST as a report is generated (#336). */
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

/** `report dual generate` with `friend` over `year` (0 for all the years), a POST as a report is generated. */
export const getDualReport = (friend: string, year: number, signal?: AbortSignal) => call<DualReport>('POST', 'report/dual/generate', { friend, year }, signal);

// each kind's label, shown through t(); a kind without one is 「[消息]」
const LABELS: Partial<Record<MessageKind, string>> = {
  quote: msg('[引用回复]'), image: msg('[图片]'), emoji: msg('[表情]'), voice: msg('[语音]'), video: msg('[视频]'),
  file: msg('[文件]'), link: msg('[链接]'), finder: msg('[视频号]'), record: msg('[聊天记录]'), contact: msg('[名片]'),
  location: msg('[位置]'), call: msg('[通话]'), system: msg('[系统消息]'),
};
// the kinds of `money`, by the app message's kind
const MONEY: Record<string, string> = { transfer: msg('[转账]'), 'red-packet': msg('[红包]'), gift: msg('[礼物]') };

/**
 * What a message of the dual report says: its text, or for anything else the label of its kind, the one the chat
 * window draws it by (an app message's from the `appMsgKind` the server gives).
 */
export function messageText(m: DualMessage): string {
  if (!m.localType || m.localType === 1) return m.content;
  const kind = kindOf(m);
  const label = kind === 'money' ? MONEY[m.appMsgKind ?? ''] : LABELS[kind];
  return label ? t(label) : t('[消息]');
}
