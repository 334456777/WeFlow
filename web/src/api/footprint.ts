// My footprint (#209): who I talked with and replied to, and where I was @-mentioned, over a range of days
// (`insight footprint --start --end`); its export (`export footprint`) and its AI summary (`insight footprint-summary`).
import { call, get } from './client';
import { dayOf } from '../chat/messages';

export interface FootprintSummary {
  /** People who wrote to me, those I replied to, and those I wrote to. */
  private_inbound_people: number;
  private_replied_people: number;
  private_outbound_people: number;
  /** replied ÷ inbound, 0–1. */
  private_reply_rate: number;
  mention_count: number;
  mention_group_count: number;
}

/** A stretch of one private chat without an hour's pause; seconds. */
export interface FootprintSegment {
  session_id: string;
  segment_index: number;
  start_ts: number;
  end_ts: number;
  incoming_count: number;
  outgoing_count: number;
  message_count: number;
  replied: boolean;
  /** The message to open it at. */
  anchor_local_id: number;
  anchor_create_time: number;
}

export interface FootprintMention {
  session_id: string;
  local_id: number;
  create_time: number;
  sender_username: string;
  message_content: string;
}

export interface Footprint {
  summary: FootprintSummary;
  private_sessions: { session_id: string; incoming_count: number; outgoing_count: number; replied: boolean }[];
  private_segments: FootprintSegment[];
  mentions: FootprintMention[];
  mention_groups: { session_id: string; count: number; latest_ts: number }[];
  diagnostics: { truncated: boolean };
}

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
