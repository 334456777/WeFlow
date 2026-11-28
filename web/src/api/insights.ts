// AI 见解（#210）：`serve --insight` 的见解引擎（或 `insight trigger`）写下的记录，经由子进程路由读取；
// 单条记录的日志在服务进程内读取（`/api/`）。只有当前账号的记录，最新的在前。
import { call, get } from './client';
import { msg, t } from '../i18n';
import type * as Contract from './contract.gen';

/** `insight records` 列出的一条记录（契约的类型，#430）。 */
export type InsightRecord = Contract.InsightSummary;

/** 一条记录的由来：发给模型的请求和模型的回答（`insight get`，契约的类型，#430）。 */
export type InsightLog = Contract.InsightRecordLog;

/** 有记录的一个会话及其记录数。 */
export type InsightContact = Contract.InsightContact;

/** `insight records` 的一页。 */
export type InsightPage = Contract.InsightRecords;

/** 每页的记录数。 */
export const PAGE = 50;

/** `insight records` 中符合关键词和会话的记录，从第 `offset` 条起的一页。 */
export async function getInsights(filter: { keyword?: string; session?: string | null; offset?: number }, signal?: AbortSignal): Promise<InsightPage> {
  const page = await get<Partial<InsightPage>>('insight/records', {
    keyword: filter.keyword?.trim() || undefined, session: filter.session || undefined, limit: PAGE, offset: filter.offset || undefined,
  }, signal);
  return { records: page.records ?? [], total: page.total ?? 0, todayCount: page.todayCount ?? 0, unreadCount: page.unreadCount ?? 0, contacts: page.contacts ?? [] };
}

/** 记录 `id` 的日志（`insight get`），在服务进程内读取；没有日志时为 null。 */
export async function getInsightLog(id: string, signal?: AbortSignal): Promise<InsightLog | null> {
  const found = await get<Contract.InsightLookup>('api/insight/get', { id }, signal);
  return found.record.log ?? null;
}

/** `insight mark-read`：把记录 `id` 标为已读。 */
export const markInsightRead = (id: string) => call<Contract.InsightMarked>('PUT', 'insight/mark-read', { id });

/** `insight clear --yes`：清除全部记录，或一个会话的记录；网页已经先问过用户（`InsightClear`）。 */
export const clearInsights = (session?: string | null) =>
  call<Contract.InsightCleared>('DELETE', 'insight/clear', { session: session || undefined, yes: true });

/** `insight trigger [session]`：立即为这个会话生成一条见解，不给会话时为第一个可以写见解的会话生成。 */
export const triggerInsight = (session?: string | null) =>
  call<Contract.InsightReply>('POST', 'insight/trigger', { 'session-id': session || undefined });

/** 每种 `triggerReason` 的名称，用 `msg()` 标出，显示时经过 `t()`。 */
export const REASONS: Record<string, string> = { activity: msg('聊天活跃'), silence: msg('久未联系'), test: msg('手动生成') };
/** 用网页的语言写出的 `triggerReason`；不认识的原样显示。 */
export const reasonLabel = (reason: string) => (reason in REASONS ? t(REASONS[reason]) : reason);
