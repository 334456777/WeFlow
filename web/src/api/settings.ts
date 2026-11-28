// 账号设置（setup.ts）以外的设置（#213）：AI 模型接口和缓存。在服务进程内读取（`/api/`），经由子进程路由修改。
import { call, get } from './client';
import { msg } from '../i18n';
import type * as Contract from './contract.gen';

/** 当前档案的 `config get <key>`；没有设置时为 null。 */
export async function getSetting(key: string, signal?: AbortSignal): Promise<string | null> {
  const value = await get<unknown>('api/config/get', { key }, signal);
  return value === null || value === undefined || value === '' ? null : String(value);
}

/** `config set`：设置当前档案里的键，或配置文件本身的设置。 */
export const setSetting = (key: string, value: string) => call<Contract.ConfigWritten>('PUT', 'config/set', { key, value });
/** `config unset`：去掉一个设置。 */
export const unsetSetting = (key: string) => call<Contract.ConfigWritten>('DELETE', 'config/unset', { key });

/** `insight test`：用已保存的设置向 AI 模型接口问一个简单的问题；回答请求的地址和得到的回答。 */
export const testAi = () => call<Contract.InsightReply>('POST', 'insight/test', {});

/** `insight models`：已保存的设置所指的 AI 模型接口提供的模型。 */
export const listModels = (signal?: AbortSignal) => call<Contract.ModelList>('GET', 'insight/models', {}, signal);

/**
 * AI 部分的配置键，与 `config set` 的键名一致；API key 从不读回（`config profiles` 只说明它有没有设置）。
 * `ai_insight_allow_context` 和 `ai_insight_context_count`：提问时是否带上最近的消息，以及带多少条。
 */
export const AI_KEYS = [
  'ai_model_api_format', 'ai_model_api_base_url', 'ai_model_api_model', 'ai_model_api_max_tokens', 'ai_insight_enabled',
  'ai_insight_allow_context', 'ai_insight_context_count',
] as const;

/** 为哪些私聊写见解：只为列表里的，或者为列表以外的全部。 */
export type InsightSessionMode = 'whitelist' | 'blacklist';

/** `ai_insight_filter_mode` 和 `ai_insight_filter_list`：为哪些会话写见解（#1078）。 */
export interface InsightSessions { mode: InsightSessionMode; list: string[] }

/** 列表里的会话 ID：服务端给出 JSON 数组，示例服务端给出数组的 JSON 文字；都不是时为空。 */
function sessionIds(value: unknown): string[] {
  let ids = value;
  if (typeof ids === 'string') {
    try { ids = JSON.parse(ids); } catch { return []; }
  }
  if (!Array.isArray(ids)) return [];
  return [...new Set(ids.filter((id): id is string => typeof id === 'string').map(id => id.trim()).filter(Boolean))];
}

/** 为哪些会话写见解；没有设置模式时，服务端按只为列表里的会话写见解处理。 */
export async function getInsightSessions(signal?: AbortSignal): Promise<InsightSessions> {
  const [mode, list] = await Promise.all([
    get<unknown>('api/config/get', { key: 'ai_insight_filter_mode' }, signal),
    get<unknown>('api/config/get', { key: 'ai_insight_filter_list' }, signal),
  ]);
  return { mode: String(mode ?? '').trim().toLowerCase() === 'blacklist' ? 'blacklist' : 'whitelist', list: sessionIds(list) };
}

/** 保存 `next` 与 `saved` 不同的部分：默认的模式和空列表用 `config unset` 去掉，而不是写进配置。 */
export async function setInsightSessions(next: InsightSessions, saved: InsightSessions): Promise<void> {
  if (next.mode !== saved.mode) {
    await (next.mode === 'blacklist' ? setSetting('ai_insight_filter_mode', 'blacklist') : unsetSetting('ai_insight_filter_mode'));
  }
  if (!sameSessions(next.list, saved.list)) {
    await (next.list.length ? setSetting('ai_insight_filter_list', JSON.stringify(next.list)) : unsetSetting('ai_insight_filter_list'));
  }
}

/** 两个列表里的会话是否相同，不论顺序。 */
export const sameSessions = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every(id => b.includes(id));

/** 是否没有任何会话能写见解：只允许列表里的会话，而列表为空。 */
export const noSessionAllowed = (s: InsightSessions) => s.mode === 'whitelist' && s.list.length === 0;

/** 缓存的各个部分，与 `cache clear` 接受的一致（契约的类型，#430）。 */
export type CachePartName = Contract.CachePart;

/** `cache list` 的一行。 */
export type CachePart = Contract.CacheUsage;

/** 缓存各部分的名称和说明，显示时经过 t()。 */
export const CACHE_LABELS: Record<CachePartName, { title: string; text: string }> = {
  images: { title: msg('图片'), text: msg('解密过的聊天图片。') },
  voices: { title: msg('语音'), text: msg('解码过的语音消息。') },
  emojis: { title: msg('表情'), text: msg('下载过的表情。') },
  sns: { title: msg('朋友圈'), text: msg('朋友圈的图片和视频。') },
  analytics: { title: msg('统计'), text: msg('聊天统计的汇总，下次统计时重新计算。') },
  api: { title: msg('网页的媒体和文件'), text: msg('网页里看过的媒体、推送的头像，以及任务中心里可以下载的文件。') },
  keys: { title: msg('密钥指纹'), text: msg('校验通过的密钥；清理后，下一条命令会重新校验。') },
  runtime: { title: msg('旧的运行时'), text: msg('旧版本释放的运行时目录和安装的 ffmpeg；当前版本不再创建。') },
  web: { title: msg('网页版本'), text: msg('下载过的网页版本；正在显示的版本会保留。') },
};

/** `cache list`：缓存的各个部分及其占用的空间，在服务进程内读取。 */
export const listCache = (signal?: AbortSignal) => get<CachePart[]>('api/cache/list', {}, signal);

/** `cache clear --<part> --dry-run`：清理这一部分会删除什么，不实际删除。 */
export const planClearCache = (part: CachePartName) =>
  call<Contract.RemovalPlan>('DELETE', 'cache/clear', { [part]: true, 'dry-run': true });

/** `cache clear --<part> --yes`：删除缓存的这一部分。 */
export const clearCache = (part: CachePartName) => call<Contract.CacheCleared>('DELETE', 'cache/clear', { [part]: true, yes: true });
