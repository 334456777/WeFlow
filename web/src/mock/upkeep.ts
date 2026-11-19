// What the sample's server keeps beside the records (#896): its cache by part, as `cache` lists and clears it, kept in
// the page while it is open; and what its `/health` says.
import type { CachePart, CacheCleared, CacheUsage, RemovalPlan } from '../api/contract.gen';
import { engineRuns } from './config';

const CACHE_DIR = 'C:\\Users\\sample\\AppData\\Local\\weflow\\cache';

/** Each part's files and what they take. */
const cache = new Map<CachePart, { files: number; bytes: number }>([
  ['images', { files: 1_284, bytes: 412_090_368 }], ['voices', { files: 326, bytes: 38_797_312 }],
  ['emojis', { files: 214, bytes: 17_825_792 }], ['sns', { files: 562, bytes: 148_897_792 }],
  ['analytics', { files: 4, bytes: 2_621_440 }], ['api', { files: 41, bytes: 9_437_184 }],
  ['keys', { files: 1, bytes: 512 }], ['web', { files: 3, bytes: 5_767_168 }], ['runtime', { files: 0, bytes: 0 }],
]);

const pathOf = (part: CachePart) => CACHE_DIR + '\\' + part;

/** `cache list`: each part of the sample's cache, as it is now. */
export const listCache = (): CacheUsage[] =>
  [...cache].map(([part, use]) => ({ part, paths: use.files ? [pathOf(part)] : [], bytes: use.bytes, files: use.files }));

/** The parts a `cache clear` asks for: its flags. */
const partsOf = (params: Record<string, unknown>) => [...cache.keys()].filter(part => params[part] === true);

/** `cache clear --<part> --dry-run`, or with `--yes` the removal. */
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

/** What the sample's `/health` says: this page's API, its lowest weflow, and whether the insight engine runs. */
export const health = () => ({
  status: 'ok', apiVersion: __WEFLOW_WEB__.apiVersion, version: __WEFLOW_WEB__.minBackendVersion, data: 'sample', insight: engineRuns(), platform: 'windows' as const,
  limits: { maxTasks: 1, maxReads: 8, maxMedia: 8 },
});
