// My footprint (#209): who I talked with and replied to, and where I was @-mentioned, over a range of days
// (`insight footprint --start --end`); its export (`export footprint`) and its AI summary (`insight footprint-summary`).
import { call, get } from './client';
import type * as Contract from './contract.gen';
import { dayOf } from '../chat/messages';

/** The footprint and its parts, the contract's (#430): keys in snake_case, as the native layer writes them. */
export type Footprint = Contract.Footprint;
export type FootprintSummary = Contract.FootprintSummary;
/** A stretch of one private chat without an hour's pause; seconds. */
export type FootprintSegment = Contract.FootprintSegment;
export type FootprintMention = Contract.FootprintMention;

/** Days `YYYY-MM-DD`, both inclusive, and what to call them. */
export interface FootprintRange { from: string; to: string; label: string }

export const getFootprint = (range: FootprintRange, signal?: AbortSignal) =>
  get<Footprint>('api/insight/footprint', { start: range.from || undefined, end: range.to || undefined }, signal);

export const footprintExport = (range: FootprintRange) => ({ format: 'json', start: range.from || undefined, end: range.to || undefined });

/** The ranges offered: today, yesterday, this week and last week (weeks start on Monday). */
export function presetRanges(now = new Date()): FootprintRange[] {
  const day = (offset: number) => { const d = new Date(now); d.setDate(d.getDate() + offset); return dayOf(d.getTime() / 1000); };
  const sinceMonday = (now.getDay() + 6) % 7;
  return [
    { label: '今天', from: day(0), to: day(0) },
    { label: '昨天', from: day(-1), to: day(-1) },
    { label: '本周', from: day(-sinceMonday), to: day(0) },
    { label: '上周', from: day(-sinceMonday - 7), to: day(-sinceMonday - 1) },
  ];
}

/** `insight footprint-summary`: the AI's few sentences on the range, from the numbers and the busiest chats and groups. */
export async function summarizeFootprint(range: FootprintRange, data: Footprint, name: (session: string) => string): Promise<string> {
  const payload = {
    rangeLabel: range.label,
    summary: data.summary,
    // as the desktop app sends them: the first twelve segments, or chats, and groups
    privateSegments: (data.private_segments.length ? data.private_segments : data.private_sessions).slice(0, 12).map(s => ({
      session_id: s.session_id, displayName: name(s.session_id), incoming_count: s.incoming_count, outgoing_count: s.outgoing_count,
      message_count: 'message_count' in s ? s.message_count : s.incoming_count + s.outgoing_count, replied: s.replied,
    })),
    mentionGroups: data.mention_groups.slice(0, 12).map(g => ({ session_id: g.session_id, displayName: name(g.session_id), count: g.count })),
  };
  const answer = await call<{ success?: boolean; message?: string; insight?: string }>('POST', 'insight/footprint-summary', { 'payload-json': JSON.stringify(payload) });
  if (!answer.success || !answer.insight) throw new Error(answer.message || 'AI 没有给出总结。');
  return answer.insight;
}

export type FootprintEvent =
  | { kind: 'chat'; at: number; segment: FootprintSegment }
  | { kind: 'mention'; at: number; mention: FootprintMention };

/** The chats and the @-mentions in the order they happened. */
export function timeline(data: Footprint): FootprintEvent[] {
  return [
    ...data.private_segments.map(segment => ({ kind: 'chat' as const, at: segment.start_ts, segment })),
    ...data.mentions.map(mention => ({ kind: 'mention' as const, at: mention.create_time, mention })),
  ].sort((a, b) => a.at - b.at);
}
