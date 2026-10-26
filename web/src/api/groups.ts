// Group chat analytics (#207): `group list`, and of one group `ranking`, `hours`, `media`, one member's `member` and
// `member-messages`, each over an optional range of days.
// The answers' types are the contract's (#430).
import { get } from './client';
import type * as Contract from './contract.gen';

export type Group = Contract.GroupChat;
export type GroupRanked = Contract.GroupRank;
/** `group media`: the message types, named, with their counts. */
export type GroupMedia = Contract.GroupMedia;
/** `group member`: the member's messages counted (`activeDays` counted, not estimated), by hour, and the words and
 * emoji they use most. */
export type MemberStats = Contract.MemberAnalytics;
export type MemberPage = Contract.MemberMessages;
/** What the page shows of a member's message. */
export type MemberMessage = Pick<Contract.ChatMessage, 'messageKey' | 'localType' | 'createTime' | 'parsedContent'>;

/** Days, `YYYY-MM-DD` local time, both inclusive; empty for no bound. */
export interface DayRange { from: string; to: string }

const span = (range: DayRange) => ({ start: range.from || undefined, end: range.to || undefined });

export const getGroups = (signal?: AbortSignal) => get<Group[]>('api/group/list', {}, signal);
export const getGroupRanking = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupRanked[]>('api/group/ranking', { 'chatroom-id': group, limit: 20, ...span(range) }, signal);
export const getGroupHours = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<Contract.GroupHours>('api/group/hours', { 'chatroom-id': group, ...span(range) }, signal);
export const getGroupMedia = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupMedia>('api/group/media', { 'chatroom-id': group, ...span(range) }, signal);
export const getMember = (group: string, member: string, range: DayRange, signal?: AbortSignal) =>
  get<MemberStats>('api/group/member', { 'chatroom-id': group, username: member, ...span(range) }, signal);
export const getMemberMessages = (group: string, member: string, range: DayRange, cursor: number, signal?: AbortSignal) =>
  get<MemberPage>('api/group/member-messages', { 'chatroom-id': group, username: member, limit: 50, cursor: cursor || undefined, ...span(range) }, signal);

/** The parameters of `group export-members` and `export-member-messages`, kept on the server as jobs. */
export const membersExport = (group: string) => ({ 'chatroom-id': group });
export const memberMessagesExport = (group: string, member: string, range: DayRange) => ({ 'chatroom-id': group, username: member, ...span(range) });
