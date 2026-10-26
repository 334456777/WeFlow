// The rest of the settings (#213): the output language, the AI model API and the cache: read in the server's
// process (`/api/`), changed through the command routes.
import { call, get } from './client';

/** `config get <key>` of the current profile (or the config file, for `lang`); null when it is not set. */
export async function getSetting(key: string, signal?: AbortSignal): Promise<string | null> {
  const value = await get<unknown>('api/config/get', { key }, signal);
  return value === null || value === undefined || value === '' ? null : String(value);
}

export const setSetting = (key: string, value: string) => call<unknown>('PUT', 'config/set', { key, value });
export const unsetSetting = (key: string) => call<unknown>('DELETE', 'config/unset', { key });

/** `insight test`: asks the AI model API something small with the settings saved. */
export const testAi = () => call<unknown>('POST', 'insight/test', {});

/** The keys of the AI part, as `config set` names them; the API key is never read back (`config profiles` says it is set). */
export const AI_KEYS = ['ai_model_api_base_url', 'ai_model_api_model', 'ai_model_api_max_tokens', 'ai_insight_enabled'] as const;

export type CachePartName = 'images' | 'voices' | 'emojis' | 'sns' | 'analytics' | 'api' | 'keys' | 'runtime' | 'web';

/** A row of `cache list`. */
export interface CachePart {
  part: CachePartName;
  paths: string[];
  bytes: number;
  files: number;
}

export const CACHE_LABELS: Record<CachePartName, { title: string; text: string }> = {
  images: { title: '图片', text: '解密过的聊天图片。' },
  voices: { title: '语音', text: '解码过的语音消息。' },
  emojis: { title: '表情', text: '下载过的表情。' },
  sns: { title: '朋友圈', text: '朋友圈的图片和视频。' },
  analytics: { title: '统计', text: '聊天统计的汇总，下次统计时重新计算。' },
  api: { title: '网页的媒体和文件', text: '网页里看过的媒体、推送的头像，以及任务中心里可以下载的文件。' },
  keys: { title: '密钥指纹', text: '校验通过的密钥；清理后，下一条命令会重新校验。' },
  runtime: { title: '旧的运行时', text: '其他 WeFlow 版本释放的运行时，当前版本的保留。' },
  web: { title: '网页版本', text: '下载过的网页版本；正在用的版本需要时会重新下载。' },
};

export const listCache = (signal?: AbortSignal) => get<CachePart[]>('api/cache/list', {}, signal);

/** What `cache clear --<part>` would remove, removing nothing. */
export const planClearCache = (part: CachePartName) =>
  call<{ paths: { path: string; bytes: number }[]; bytes: number }>('DELETE', 'cache/clear', { [part]: true, 'dry-run': true });

export const clearCache = (part: CachePartName) => call<unknown>('DELETE', 'cache/clear', { [part]: true, yes: true });
