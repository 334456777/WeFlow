// The rest of the settings (#213): the AI model API and the cache: read in the server's
// process (`/api/`), changed through the command routes.
import { call, get } from './client';
import { msg } from '../i18n';
import type * as Contract from './contract.gen';

/** `config get <key>` of the current profile; null when it is not set. */
export async function getSetting(key: string, signal?: AbortSignal): Promise<string | null> {
  const value = await get<unknown>('api/config/get', { key }, signal);
  return value === null || value === undefined || value === '' ? null : String(value);
}

/** `config set`: the key in the current profile, or a setting of the file. */
export const setSetting = (key: string, value: string) => call<Contract.ConfigWritten>('PUT', 'config/set', { key, value });
/** `config unset`. */
export const unsetSetting = (key: string) => call<Contract.ConfigWritten>('DELETE', 'config/unset', { key });

/** `insight test`: asks the AI model API something small with the settings saved; says where and what came back. */
export const testAi = () => call<Contract.InsightReply>('POST', 'insight/test', {});

/** `insight models`: the models the AI model API of the saved settings offers. */
export const listModels = (signal?: AbortSignal) => call<Contract.ModelList>('GET', 'insight/models', {}, signal);

/**
 * The keys of the AI part, as `config set` names them; the API key is never read back (`config profiles` says it is set).
 * `ai_insight_allow_context` and `ai_insight_context_count`: whether the recent messages go with the question, and how
 * many.
 */
export const AI_KEYS = [
  'ai_model_api_format', 'ai_model_api_base_url', 'ai_model_api_model', 'ai_model_api_max_tokens', 'ai_insight_enabled',
  'ai_insight_allow_context', 'ai_insight_context_count',
] as const;

/** Which private chats the insights are written for: only those of the list, or all but them. */
export type InsightSessionMode = 'whitelist' | 'blacklist';

/** `ai_insight_filter_mode` and `ai_insight_filter_list`, the conversations the insights are written for (#1078). */
export interface InsightSessions { mode: InsightSessionMode; list: string[] }

/** The ids of a list as the server keeps it (a JSON array) or as the sample server does (its text); none otherwise. */
function sessionIds(value: unknown): string[] {
  let ids = value;
  if (typeof ids === 'string') {
    try { ids = JSON.parse(ids); } catch { return []; }
  }
  if (!Array.isArray(ids)) return [];
  return [...new Set(ids.filter((id): id is string => typeof id === 'string').map(id => id.trim()).filter(Boolean))];
}

/** The conversations the insights are written for; the server reads an unset mode as the list's alone. */
export async function getInsightSessions(signal?: AbortSignal): Promise<InsightSessions> {
  const [mode, list] = await Promise.all([
    get<unknown>('api/config/get', { key: 'ai_insight_filter_mode' }, signal),
    get<unknown>('api/config/get', { key: 'ai_insight_filter_list' }, signal),
  ]);
  return { mode: String(mode ?? '').trim().toLowerCase() === 'blacklist' ? 'blacklist' : 'whitelist', list: sessionIds(list) };
}

/** Saves what of `next` differs from `saved`: the default mode and an empty list are unset rather than written. */
export async function setInsightSessions(next: InsightSessions, saved: InsightSessions): Promise<void> {
  if (next.mode !== saved.mode) {
    await (next.mode === 'blacklist' ? setSetting('ai_insight_filter_mode', 'blacklist') : unsetSetting('ai_insight_filter_mode'));
  }
  if (!sameSessions(next.list, saved.list)) {
    await (next.list.length ? setSetting('ai_insight_filter_list', JSON.stringify(next.list)) : unsetSetting('ai_insight_filter_list'));
  }
}

/** Whether two lists hold the same conversations, in any order. */
export const sameSessions = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every(id => b.includes(id));

/** Whether no conversation may have an insight: the list alone is allowed, and it is empty. */
export const noSessionAllowed = (s: InsightSessions) => s.mode === 'whitelist' && s.list.length === 0;

/** The parts of the cache, as `cache clear` takes them (the contract's, #430). */
export type CachePartName = Contract.CachePart;

/** A row of `cache list`. */
export type CachePart = Contract.CacheUsage;

/** Shown through t(). */
export const CACHE_LABELS: Record<CachePartName, { title: string; text: string }> = {
  images: { title: msg('图片'), text: msg('解密过的聊天图片。') },
  voices: { title: msg('语音'), text: msg('解码过的语音消息。') },
  emojis: { title: msg('表情'), text: msg('下载过的表情。') },
  sns: { title: msg('朋友圈'), text: msg('朋友圈的图片和视频。') },
  analytics: { title: msg('统计'), text: msg('聊天统计的汇总，下次统计时重新计算。') },
  api: { title: msg('网页的媒体和文件'), text: msg('网页里看过的媒体、推送的头像，以及任务中心里可以下载的文件。') },
  keys: { title: msg('密钥指纹'), text: msg('校验通过的密钥；清理后，下一条命令会重新校验。') },
  runtime: { title: msg('旧的运行时'), text: msg('旧版本释放的运行时目录和安装的 ffmpeg；当前版本不再创建。') },
  web: { title: msg('网页版本'), text: msg('下载过的网页版本；正在用的版本需要时会重新下载。') },
};

/** `cache list`: each part of the cache and what it takes, read in the server's process. */
export const listCache = (signal?: AbortSignal) => get<CachePart[]>('api/cache/list', {}, signal);

/** What `cache clear --<part>` would remove, removing nothing. */
export const planClearCache = (part: CachePartName) =>
  call<Contract.RemovalPlan>('DELETE', 'cache/clear', { [part]: true, 'dry-run': true });

/** `cache clear --<part> --yes`: removes that part of the cache. */
export const clearCache = (part: CachePartName) => call<Contract.CacheCleared>('DELETE', 'cache/clear', { [part]: true, yes: true });
