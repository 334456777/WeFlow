// 年度报告（#208）：`report annual years` 和 `report annual generate`，统计一年或全部年份；
// 以及双人报告 `report dual generate`。
import { call, get } from './client';
import { msg, t } from '../i18n';
import { kindOf, type MessageKind } from '../messageKind';
import type * as Contract from './contract.gen';

/** `report annual generate` 的回答，类型是契约的（#430）。 */
export type AnnualReport = Contract.AnnualReport;

/** `report annual years`：有私聊消息的年份，在服务进程内读取。 */
export const getReportYears = (signal?: AbortSignal) => get<Contract.ReportYears>('api/report/annual/years', {}, signal);
/** `report annual generate`：生成 `year` 年（0 为全部年份）的年度报告；报告是生成出来的，所以用 POST（#336）。 */
export const getAnnualReport = (year: number, signal?: AbortSignal) => call<AnnualReport>('POST', 'report/annual/generate', { year }, signal);

/** 把以秒为单位的等待时长写成便于阅读的形式：几秒、几分钟或几小时。 */
export function formatWait(seconds: number): string {
  if (seconds < 60) return t('{n} 秒', { n: Math.max(0, Math.round(seconds)) });
  if (seconds < 3600) return t('{n} 分钟', { n: Math.round(seconds / 60) });
  const hours = seconds / 3600;
  return t('{n} 小时', { n: hours < 10 ? Math.round(hours * 10) / 10 : Math.round(hours) });
}

/** 一周中最忙的那个小时，如「周六 21 点」；没有消息时为 null。 */
export function busiestHour(data: number[][]): { weekday: number; hour: number; count: number } | null {
  let best: { weekday: number; hour: number; count: number } | null = null;
  data.forEach((row, weekday) => row.forEach((count, hour) => { if (count > 0 && (!best || count > best.count)) best = { weekday, hour, count }; }));
  return best;
}

/** 双人报告里的一条消息；`createTime` 以毫秒为单位，不是文字的消息的 `content` 是原文。 */
export type DualMessage = Contract.DualMessage;

/** `report dual generate`：我和一位好友在一年或全部年份里的双人报告。 */
export type DualReport = Contract.DualReport;

/** `report dual generate`：生成与 `friend` 在 `year` 年（0 为全部年份）的双人报告，同样用 POST。 */
export const getDualReport = (friend: string, year: number, signal?: AbortSignal) => call<DualReport>('POST', 'report/dual/generate', { friend, year }, signal);

// 每种消息的标签，显示时经过 t()；没有标签的种类显示「[消息]」
const LABELS: Partial<Record<MessageKind, string>> = {
  quote: msg('[引用回复]'), image: msg('[图片]'), emoji: msg('[表情]'), voice: msg('[语音]'), video: msg('[视频]'),
  file: msg('[文件]'), link: msg('[链接]'), finder: msg('[视频号]'), record: msg('[聊天记录]'), contact: msg('[名片]'),
  location: msg('[位置]'), call: msg('[通话]'), system: msg('[系统消息]'),
};
// `money` 种类的标签，按 app 消息的种类区分
const MONEY: Record<string, string> = { transfer: msg('[转账]'), 'red-packet': msg('[红包]'), gift: msg('[礼物]') };

/**
 * 双人报告里一条消息显示的文字：文字消息显示它的文字，其他消息显示种类的标签；
 * 种类与聊天窗口画它时用的一致（app 消息的种类按服务端给出的 `appMsgKind` 判定）。
 */
export function messageText(m: DualMessage): string {
  if (!m.localType || m.localType === 1) return m.content;
  const kind = kindOf(m);
  const label = kind === 'money' ? MONEY[m.appMsgKind ?? ''] : LABELS[kind];
  return label ? t(label) : t('[消息]');
}
