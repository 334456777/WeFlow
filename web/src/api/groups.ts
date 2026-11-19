// Group chat analytics (#207): `group list`, and of one group `ranking`, `hours`, `media`, one member's `member` and
// `member-messages`, each over an optional range of days.
// The answers' types are the contract's (#430).
import { get } from './client';
import type * as Contract from './contract.gen';

/** A group chat of `group list`. */
export type Group = Contract.GroupChat;
/** A member of `group ranking` and their messages. */
export type GroupRanked = Contract.GroupRank;
/** `group media`: the message types, named, with their counts. */
export type GroupMedia = Contract.GroupMedia;
/** `group member`: the member's messages counted (`activeDays` counted, not estimated), by hour, and the words and
 * emoji they use most. */
export type MemberStats = Contract.MemberAnalytics;
/** A page of `group member-messages`. */
export type MemberPage = Contract.MemberMessages;
/** What the page shows of a member's message. */
export type MemberMessage = Pick<Contract.ChatMessage, 'messageKey' | 'localType' | 'createTime' | 'parsedContent'>;

/** Days, `YYYY-MM-DD` local time, both inclusive; empty for no bound. */
export interface DayRange { from: string; to: string }

const span = (range: DayRange) => ({ start: range.from || undefined, end: range.to || undefined });

/** `group list`, the largest first. */
export const getGroups = (signal?: AbortSignal) => get<Group[]>('api/group/list', {}, signal);
/** `group ranking`: the 20 members with the most messages in the range. */
export const getGroupRanking = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupRanked[]>('api/group/ranking', { 'chatroom-id': group, limit: 20, ...span(range) }, signal);
/** `group hours`: the group's messages by hour in the range. */
export const getGroupHours = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<Contract.GroupHours>('api/group/hours', { 'chatroom-id': group, ...span(range) }, signal);
/** `group media` in the range. */
export const getGroupMedia = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupMedia>('api/group/media', { 'chatroom-id': group, ...span(range) }, signal);
/** `group member` in the range. */
export const getMember = (group: string, member: string, range: DayRange, signal?: AbortSignal) =>
  get<MemberStats>('api/group/member', { 'chatroom-id': group, username: member, ...span(range) }, signal);
/** A page of 50 of a member's messages in the range, from `cursor` (0 for the first page). */
export const getMemberMessages = (group: string, member: string, range: DayRange, cursor: number, signal?: AbortSignal) =>
  get<MemberPage>('api/group/member-messages', { 'chatroom-id': group, username: member, limit: 50, cursor: cursor || undefined, ...span(range) }, signal);

/** The parameters of `group export-members` and `export-member-messages`, kept on the server as jobs. */
export const membersExport = (group: string) => ({ 'chatroom-id': group });
/** The parameters of `group export-member-messages` over the range. */
export const memberMessagesExport = (group: string, member: string, range: DayRange) => ({ 'chatroom-id': group, username: member, ...span(range) });
