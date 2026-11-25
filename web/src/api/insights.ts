// The AI insights (#210): the records the insight engine of `serve --insight` (or `insight trigger`) wrote, through
// the command routes; one record's log is read in the server's process (`/api/`). The records of the current account
// only, newest first.
import { call, get } from './client';
import { msg, t } from '../i18n';
import type * as Contract from './contract.gen';

/** A record as `insight records` lists it (the contract's, #430). */
export type InsightRecord = Contract.InsightSummary;

/** How a record came to be: the request to the model and its answer (`insight get`, the contract's, #430). */
export type InsightLog = Contract.InsightRecordLog;

/** A conversation with records, and how many. */
export type InsightContact = Contract.InsightContact;

/** A page of `insight records`. */
export type InsightPage = Contract.InsightRecords;

/** Records per page. */
export const PAGE = 50;

/** A page of `insight records` that the keyword and the conversation match, from `offset`. */
export async function getInsights(filter: { keyword?: string; session?: string | null; offset?: number }, signal?: AbortSignal): Promise<InsightPage> {
  const page = await get<Partial<InsightPage>>('insight/records', {
    keyword: filter.keyword?.trim() || undefined, session: filter.session || undefined, limit: PAGE, offset: filter.offset || undefined,
  }, signal);
  return { records: page.records ?? [], total: page.total ?? 0, todayCount: page.todayCount ?? 0, unreadCount: page.unreadCount ?? 0, contacts: page.contacts ?? [] };
}

/** The log of the record `id` (`insight get`), read in the server's process; null when it has none. */
export async function getInsightLog(id: string, signal?: AbortSignal): Promise<InsightLog | null> {
  const found = await get<Contract.InsightLookup>('api/insight/get', { id }, signal);
  return found.record.log ?? null;
}

/** `insight mark-read`. */
export const markInsightRead = (id: string) => call<Contract.InsightMarked>('PUT', 'insight/mark-read', { id });

/** `insight clear --yes`: every record, or one conversation's; the page has asked first (`InsightClear`). */
export const clearInsights = (session?: string | null) =>
  call<Contract.InsightCleared>('DELETE', 'insight/clear', { session: session || undefined, yes: true });

/** `insight trigger [session]`: one insight now, for that conversation or the first one it may write for. */
export const triggerInsight = (session?: string | null) =>
  call<Contract.InsightReply>('POST', 'insight/trigger', { 'session-id': session || undefined });

/** What each `triggerReason` is called, marked with `msg()` and shown through `t()`. */
export const REASONS: Record<string, string> = { activity: msg('聊天活跃'), silence: msg('久未联系'), test: msg('手动生成') };
/** A `triggerReason` in the page's language; one it does not know as it is. */
export const reasonLabel = (reason: string) => (reason in REASONS ? t(REASONS[reason]) : reason);
