// 示例服务端在记录之外保存的东西（#896）：按部分划分的缓存，按 `cache` 的方式列出和清理，页面打开期间保存在页面里；
// 以及它的 `/health` 的回答。
import type { CachePart, CacheCleared, CacheUsage, RemovalPlan } from '../api/contract.gen';
import { engineRuns } from './config';

const CACHE_DIR = 'C:\\Users\\sample\\AppData\\Local\\weflow\\cache';

/** 每一部分的文件数和占用的空间。 */
const cache = new Map<CachePart, { files: number; bytes: number }>([
  ['images', { files: 1_284, bytes: 412_090_368 }], ['voices', { files: 326, bytes: 38_797_312 }],
  ['emojis', { files: 214, bytes: 17_825_792 }], ['sns', { files: 562, bytes: 148_897_792 }],
  ['analytics', { files: 4, bytes: 2_621_440 }], ['api', { files: 41, bytes: 9_437_184 }],
  ['keys', { files: 1, bytes: 512 }], ['web', { files: 3, bytes: 5_767_168 }], ['runtime', { files: 0, bytes: 0 }],
]);

const pathOf = (part: CachePart) => CACHE_DIR + '\\' + part;

/** `cache list`：示例缓存的各个部分此刻的状态。 */
export const listCache = (): CacheUsage[] =>
  [...cache].map(([part, use]) => ({ part, paths: use.files ? [pathOf(part)] : [], bytes: use.bytes, files: use.files }));

/** `cache clear` 要清理的部分，即它带的选项。 */
const partsOf = (params: Record<string, unknown>) => [...cache.keys()].filter(part => params[part] === true);

/** `cache clear --<part> --dry-run`；带 `--yes` 时实际删除。 */
export function clearCache(params: Record<string, unknown>): RemovalPlan | CacheCleared {
  const parts = partsOf(params).filter(part => cache.get(part)!.files > 0);
  if (params['dry-run'] === true) {
    const paths = parts.map(part => ({ path: pathOf(part), bytes: cache.get(part)!.bytes }));
    return { dryRun: true, paths, bytes: paths.reduce((n, p) => n + p.bytes, 0), profileKeys: [] };
  }
  const freedBytes = parts.reduce((n, part) => n + cache.get(part)!.bytes, 0);
  for (const part of parts) cache.set(part, { files: 0, bytes: 0 });
  return { removedPaths: parts.map(pathOf), freedBytes };
}

/** 示例服务端的 `/health` 的回答：这个网页的 API 版本、它要求的最低 weflow 版本，以及见解引擎是否运行。 */
export const health = () => ({
  status: 'ok', apiVersion: __WEFLOW_WEB__.apiVersion, version: __WEFLOW_WEB__.minBackendVersion, data: 'sample', insight: engineRuns(), platform: 'windows' as const,
  limits: { maxTasks: 1, maxReads: 8, maxMedia: 8 },
});
