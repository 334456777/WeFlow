// 我的足迹（#209）：一段日期内和谁聊过、回复过谁、在哪里被 @（`insight footprint --start --end`）；
// 以及足迹的导出（`export footprint`）和 AI 总结（`insight footprint-summary`）。
import { call, get } from './client';
import { t } from '../i18n';
import type * as Contract from './contract.gen';
import { dayOf } from '../format';

/** 足迹及其各部分，契约的类型（#430）：键是 snake_case，与只读数据库层写出的一致。 */
export type Footprint = Contract.Footprint;
/** 足迹的各项计数。 */
export type FootprintSummary = Contract.FootprintSummary;
/** 一个私聊里中间没有超过一小时停顿的一段聊天；时间以秒为单位。 */
export type FootprintSegment = Contract.FootprintSegment;
/** 群里 @ 我的一条消息。 */
export type FootprintMention = Contract.FootprintMention;

/** 日期范围 `YYYY-MM-DD`（两端都包含）及其名称。 */
export interface FootprintRange { from: string; to: string; label: string }

/** 日期范围内的 `insight footprint`，在服务进程内读取。 */
export const getFootprint = (range: FootprintRange, signal?: AbortSignal) =>
  get<Footprint>('api/insight/footprint', { start: range.from || undefined, end: range.to || undefined }, signal);

/** `export footprint` 写出的格式（#390）：统计写成 JSON，或者时间线写成表格的行。 */
export const FOOTPRINT_FORMATS = [
  { value: 'json', label: 'JSON' }, { value: 'csv', label: 'CSV' }, { value: 'excel', label: 'Excel' },
] as const;
/** `export footprint` 的一种格式。 */
export type FootprintFormat = (typeof FOOTPRINT_FORMATS)[number]['value'];

/** 日期范围内的 `export footprint` 的参数。 */
export const footprintExport = (range: FootprintRange, format: FootprintFormat = 'json') =>
  ({ format, start: range.from || undefined, end: range.to || undefined });

/** 按名称提供的日期范围：今天、昨天、本周和上周（每周从周一开始）。 */
export const PRESETS = ['today', 'yesterday', 'thisWeek', 'lastWeek'] as const;
/** 按名称提供的一个日期范围。 */
export type FootprintPreset = (typeof PRESETS)[number];

/**
 * 选定的日期范围：预设的一种，每次显示时按当天和网页的语言算出它的日期和名称；或者自己选的日期。
 */
export type FootprintChoice = { preset: FootprintPreset } | { from: string; to: string };

/** 提供的各个日期范围，日期按 `now` 计算。 */
export function presetRanges(now = new Date()): (FootprintRange & { preset: FootprintPreset })[] {
  const day = (offset: number) => { const d = new Date(now); d.setDate(d.getDate() + offset); return dayOf(d.getTime() / 1000); };
  const sinceMonday = (now.getDay() + 6) % 7;
  return [
    { preset: 'today', label: t('今天'), from: day(0), to: day(0) },
    { preset: 'yesterday', label: t('昨天'), from: day(-1), to: day(-1) },
    { preset: 'thisWeek', label: t('本周'), from: day(-sinceMonday), to: day(0) },
    { preset: 'lastWeek', label: t('上周'), from: day(-sinceMonday - 7), to: day(-sinceMonday - 1) },
  ];
}

/** 选定范围的日期和名称，按 `now` 计算。 */
export function rangeOf(choice: FootprintChoice, now = new Date()): FootprintRange {
  if ('preset' in choice) {
    const { label, from, to } = presetRanges(now).find(p => p.preset === choice.preset)!;
    return { label, from, to };
  }
  const { from, to } = choice;
  return { from, to, label: from === to ? from : t('{from} 至 {to}', { from, to }) };
}

/** `insight footprint-summary`：AI 根据计数和最活跃的私聊、群为这段日期写的几句总结。 */
export async function summarizeFootprint(range: FootprintRange, data: Footprint, name: (session: string) => string): Promise<string> {
  // 用足迹本身的键：前 12 段聊天（没有分段时为前 12 个私聊）和前 12 个群，各带名称
  const payload = {
    range_label: range.label,
    summary: data.summary,
    private_segments: (data.private_segments.length ? data.private_segments : data.private_sessions).slice(0, 12).map(s => ({
      session_id: s.session_id, display_name: name(s.session_id), incoming_count: s.incoming_count, outgoing_count: s.outgoing_count,
      message_count: 'message_count' in s ? s.message_count : s.incoming_count + s.outgoing_count, replied: s.replied,
    })),
    mention_groups: data.mention_groups.slice(0, 12).map(g => ({ session_id: g.session_id, display_name: name(g.session_id), count: g.count })),
  };
  const answer = await call<Contract.InsightReply>('POST', 'insight/footprint-summary', { 'payload-json': JSON.stringify(payload) });
  if (!answer.insight) throw new Error(answer.message || t('AI 没有给出总结。'));
  return answer.insight;
}

/** 足迹时间线的一行：一段聊天或一次 @，以及它的时间（Unix 秒数）。 */
export type FootprintEvent =
  | { kind: 'chat'; at: number; segment: FootprintSegment }
  | { kind: 'mention'; at: number; mention: FootprintMention };

/** 按发生的先后排列的聊天和 @。 */
export function timeline(data: Footprint): FootprintEvent[] {
  return [
    ...data.private_segments.map(segment => ({ kind: 'chat' as const, at: segment.start_ts, segment })),
    ...data.mentions.map(mention => ({ kind: 'mention' as const, at: mention.create_time, mention })),
  ].sort((a, b) => a.at - b.at);
}
