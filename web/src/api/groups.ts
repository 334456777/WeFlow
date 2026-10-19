// Group chat analytics (#207): `group list`, and of one group `ranking`, `hours`, `media`, one member's `member` and
// `member-messages`, each over an optional range of days.
import { get } from './client';
import type { Overall } from './analytics';

export interface Group { username: string; displayName: string; memberCount: number }

export interface GroupRanked { member: { username: string; displayName: string; avatarUrl?: string }; messageCount: number }

/** `group media`: the message types, named, with their counts. */
export interface GroupMedia { typeCounts: { type: number; name: string; count: number }[]; total: number }

/** `group member`: the member's messages counted, by hour, and the words and emoji they use most. */
export interface MemberStats {
  /** `activeDays` here is counted, not estimated. */
  statistics: Overall;
  timeDistribution: Record<string, number>;
  commonPhrases: { phrase: string; count: number }[];
  commonEmojis: { emoji: string; count: number }[];
}

export interface MemberMessage { messageKey: string; localType: number; createTime: number; parsedContent: string }
export interface MemberPage { messages: MemberMessage[]; hasMore: boolean; nextCursor: number }

/** Days, `YYYY-MM-DD` local time, both inclusive; empty for no bound. */
export interface DayRange { from: string; to: string }

const span = (range: DayRange) => ({ start: range.from || undefined, end: range.to || undefined });

export const getGroups = (signal?: AbortSignal) => get<Group[]>('group/list', {}, signal);
export const getGroupRanking = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupRanked[]>('group/ranking', { 'chatroom-id': group, limit: 20, ...span(range) }, signal);
export const getGroupHours = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<{ hourlyDistribution: Record<string, number> }>('group/hours', { 'chatroom-id': group, ...span(range) }, signal);
export const getGroupMedia = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupMedia>('group/media', { 'chatroom-id': group, ...span(range) }, signal);
export const getMember = (group: string, member: string, range: DayRange, signal?: AbortSignal) =>
  get<MemberStats>('group/member', { 'chatroom-id': group, username: member, ...span(range) }, signal);
export const getMemberMessages = (group: string, member: string, range: DayRange, cursor: number, signal?: AbortSignal) =>
  get<MemberPage>('group/member-messages', { 'chatroom-id': group, username: member, limit: 50, cursor: cursor || undefined, ...span(range) }, signal);

/** The parameters of `group export-members` and `export-member-messages`, kept on the server as jobs. */
export const membersExport = (group: string) => ({ 'chatroom-id': group });
export const memberMessagesExport = (group: string, member: string, range: DayRange) => ({ 'chatroom-id': group, username: member, ...span(range) });
