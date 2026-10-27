// The AI insights (#210): the records the insight engine of `serve --insight` (or `insight trigger`) wrote, through
// the command routes; one record's log is read in the server's process (`/api/`). The records of the current account
// only, newest first.
import { call, get } from './client';
import type * as Contract from './contract.gen';

/** A record as `insight records` lists it (the contract's, #430). */
export type InsightRecord = Contract.InsightSummary;

/** How a record came to be: the request to the model and its answer (`insight get`, the contract's, #430). */
export type InsightLog = Contract.InsightRecordLog;

export type InsightContact = Contract.InsightContact;

/** A page of `insight records`, without the desktop app's `success`. */
export type InsightPage = Omit<Contract.InsightRecords, 'success'>;

export const PAGE = 50;

export async function getInsights(filter: { keyword?: string; session?: string | null; offset?: number }, signal?: AbortSignal): Promise<InsightPage> {
  const page = await get<Partial<InsightPage>>('insight/records', {
    keyword: filter.keyword?.trim() || undefined, session: filter.session || undefined, limit: PAGE, offset: filter.offset || undefined,
  }, signal);
  return { records: page.records ?? [], total: page.total ?? 0, todayCount: page.todayCount ?? 0, unreadCount: page.unreadCount ?? 0, contacts: page.contacts ?? [] };
}

export async function getInsightLog(id: string, signal?: AbortSignal): Promise<InsightLog | null> {
  const found = await get<Contract.InsightLookup>('api/insight/get', { id }, signal);
  return found.record?.log ?? null;
}

export const markInsightRead = (id: string) => call<Contract.InsightMarked>('PUT', 'insight/mark-read', { id });

/** `insight clear`: every record, or one conversation's. */
export const clearInsights = (session?: string | null) => call<Contract.InsightCleared>('DELETE', 'insight/clear', { session: session || undefined });

/** `insight trigger [session]`: one insight now, for that conversation or the first one it may write for. */
export const triggerInsight = (session?: string | null) =>
  call<Contract.InsightReply>('POST', 'insight/trigger', { 'session-id': session || undefined });

export const REASONS: Record<string, string> = { activity: '聊天活跃', silence: '久未联系', test: '手动生成' };
export const reasonLabel = (reason: string) => REASONS[reason] ?? reason;
