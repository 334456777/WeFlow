// What the sample's server keeps beside the records (#896): its cache by part, and the WebUI versions it has, as
// `cache` and `web` list and change them; and what its `/health` says. Kept in the page while it is open.
import type { CachePart, CacheCleared, CacheUsage, RemovalPlan, WebChosen, WebInstalled, WebPruned, WebPublished } from '../api/contract.gen';
import { engineRuns } from './config';

const CACHE_DIR = 'C:\\Users\\sample\\AppData\\Local\\weflow\\cache';
const WEB_DIR = 'C:\\Users\\sample\\AppData\\Local\\weflow\\web';

/** Each part's files and what they take. */
const cache = new Map<CachePart, { files: number; bytes: number }>([
  ['images', { files: 1_284, bytes: 412_090_368 }], ['voices', { files: 326, bytes: 38_797_312 }],
  ['emojis', { files: 214, bytes: 17_825_792 }], ['sns', { files: 562, bytes: 148_897_792 }],
  ['analytics', { files: 4, bytes: 2_621_440 }], ['api', { files: 41, bytes: 9_437_184 }],
  ['keys', { files: 1, bytes: 512 }], ['web', { files: 3, bytes: 5_767_168 }], ['runtime', { files: 0, bytes: 0 }],
]);

const pathOf = (part: CachePart) => CACHE_DIR + '\\' + part;

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

const WEB = __WEFLOW_WEB__;

/** The versions published, newest first: this page's, and older ones written for the same API. */
const PUBLISHED: WebPublished[] = [
  { version: WEB.version, size: 1_942_016, apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion, suits: true, why: null, downloaded: true },
  { version: '0.4.0', size: 1_871_872, apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion, suits: true, why: null, downloaded: true },
  { version: '0.3.0', size: 1_703_936, apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion, suits: true, why: null, downloaded: false },
].filter((v, i, all) => all.findIndex(o => o.version === v.version) === i);

const kept = new Set(PUBLISHED.filter(v => v.downloaded).map(v => v.version));
let picked: string | null = null;

export function listWeb(remote: boolean): WebInstalled[] | WebPublished[] {
  if (remote) return PUBLISHED.map(v => ({ ...v, downloaded: kept.has(v.version) }));
  return PUBLISHED.filter(v => kept.has(v.version)).map(v => ({
    version: v.version, apiVersion: v.apiVersion ?? WEB.apiVersion, minBackendVersion: v.minBackendVersion ?? WEB.minBackendVersion,
    path: WEB_DIR + '\\' + v.version, inUse: v.version === WEB.version, picked: v.version === picked, suits: v.suits,
  }));
}

/** `web use <version>`: downloads it when needed and picks it; the page shown changes on the next start. */
export function pickWeb(version: string): WebChosen | null {
  if (!PUBLISHED.some(v => v.version === version)) return null;
  kept.add(version);
  picked = version;
  return { version, path: WEB_DIR + '\\' + version };
}

/** `web prune`: removes the versions neither shown nor picked. */
export function pruneWeb(): WebPruned {
  const removed = [...kept].filter(v => v !== WEB.version && v !== picked);
  for (const version of removed) kept.delete(version);
  return { removed, kept: [...kept], shown: [WEB.version] };
}

/** What the sample's `/health` says: this page's API, its lowest weflow, and whether the insight engine runs. */
export const health = () => ({
  status: 'ok', apiVersion: WEB.apiVersion, version: WEB.minBackendVersion, data: 'sample', insight: engineRuns(), platform: 'windows' as const,
  limits: { maxTasks: 1, maxReads: 8, maxMedia: 8 },
});
