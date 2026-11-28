// 群聊分析（#207）：`group list`，以及一个群的 `ranking`、`hours`、`media`，一个成员的 `member` 和
// `member-messages`，都可以限定日期范围。回答的类型是契约的类型（#430）。
import { get } from './client';
import type * as Contract from './contract.gen';

/** `group list` 的一个群聊。 */
export type Group = Contract.GroupChat;
/** `group ranking` 的一个成员及其消息数。 */
export type GroupRanked = Contract.GroupRank;
/** `group media`：各种消息类型的名称和条数。 */
export type GroupMedia = Contract.GroupMedia;
/** `group member`：成员的消息数和发过消息的天数（`activeDays`）、按小时的分布，以及最常用的词和表情。 */
export type MemberStats = Contract.MemberAnalytics;
/** `group member-messages` 的一页。 */
export type MemberPage = Contract.MemberMessages;
/** 网页显示的一条成员消息的字段。 */
export type MemberMessage = Pick<Contract.ChatMessage, 'messageKey' | 'localType' | 'createTime' | 'parsedContent'>;

/** 日期范围：`YYYY-MM-DD`，本地时间，两端都包含；为空表示不限。 */
export interface DayRange { from: string; to: string }

const span = (range: DayRange) => ({ start: range.from || undefined, end: range.to || undefined });

/** `group list`：群聊列表，成员最多的在前。 */
export const getGroups = (signal?: AbortSignal) => get<Group[]>('api/group/list', {}, signal);
/** `group ranking`：日期范围内消息最多的 20 个成员。 */
export const getGroupRanking = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupRanked[]>('api/group/ranking', { 'chatroom-id': group, limit: 20, ...span(range) }, signal);
/** `group hours`：日期范围内群消息按小时的分布。 */
export const getGroupHours = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<Contract.GroupHours>('api/group/hours', { 'chatroom-id': group, ...span(range) }, signal);
/** 日期范围内的 `group media`。 */
export const getGroupMedia = (group: string, range: DayRange, signal?: AbortSignal) =>
  get<GroupMedia>('api/group/media', { 'chatroom-id': group, ...span(range) }, signal);
/** 日期范围内的 `group member`。 */
export const getMember = (group: string, member: string, range: DayRange, signal?: AbortSignal) =>
  get<MemberStats>('api/group/member', { 'chatroom-id': group, username: member, ...span(range) }, signal);
/** `group member-messages`：日期范围内一个成员从 `cursor` 起的一页消息，每页 50 条（第一页的 `cursor` 为 0）。 */
export const getMemberMessages = (group: string, member: string, range: DayRange, cursor: number, signal?: AbortSignal) =>
  get<MemberPage>('api/group/member-messages', { 'chatroom-id': group, username: member, limit: 50, cursor: cursor || undefined, ...span(range) }, signal);

/** `group export-members` 的参数。它和 `group export-member-messages` 都在服务端作为作业运行。 */
export const membersExport = (group: string) => ({ 'chatroom-id': group });
/** 限定日期范围的 `group export-member-messages` 的参数。 */
export const memberMessagesExport = (group: string, member: string, range: DayRange) => ({ 'chatroom-id': group, username: member, ...span(range) });
