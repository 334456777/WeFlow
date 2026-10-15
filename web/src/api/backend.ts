// Whether `weflow serve` is there and speaks the API this build was written for. `GET /health` needs no token:
// `{"status": "ok", "apiVersion": "1", "version": "5.0.0"}` (docs/serve-api.md, "Versions").
import { useCallback, useEffect, useState } from 'react';

export interface Health {
  status: string;
  apiVersion: string;
  version: string;
}

export type Backend =
  | { state: 'checking' }
  | { state: 'offline' }
  | { state: 'ready'; health: Health }
  | { state: 'incompatible'; health: Health; reason: string };

export const WEB = __WEFLOW_WEB__;

type Identifier = number | string;

function parse(version: string): { core: number[]; pre: Identifier[] } | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(version.trim());
  if (!match) return null;
  const pre = match[4] ? match[4].split('.').map(p => (/^\d+$/.test(p) ? Number(p) : p)) : [];
  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], pre };
}

/** A development build reports `git describe`: the last tag, then the commits since it (`5.0.0-rc.4-3-gabc1234`). */
function described(version: string): { tag: string; ahead: number } {
  const match = /^(.+)-(\d+)-g[0-9a-f]+$/.exec(version.trim());
  return match ? { tag: match[1], ahead: Number(match[2]) } : { tag: version, ahead: 0 };
}

/**
 * Orders two versions as SemVer does: a pre-release comes before its release, numeric identifiers before
 * others. A `git describe` version sorts right after its tag and before the next one. Returns NaN when either is
 * not a version.
 */
export function compareVersions(a: string, b: string): number {
  const p = described(a);
  const q = described(b);
  const order = compareTags(p.tag, q.tag);
  return order === 0 ? p.ahead - q.ahead : order;
}

function compareTags(a: string, b: string): number {
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return NaN;
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i];
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length;
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i];
    const q = y.pre[i];
    if (p === q) continue;
    if (typeof p === 'number' && typeof q === 'number') return p - q;
    if (typeof p === 'number') return -1;
    if (typeof q === 'number') return 1;
    return p < q ? -1 : 1;
  }
  return x.pre.length - y.pre.length;
}

/** Why this build cannot use the server, or null when it can. */
export function incompatibility(health: Health, web = WEB): string | null {
  if (health.apiVersion !== web.apiVersion) {
    return '网页需要 API 版本 ' + web.apiVersion + '，weflow 提供的是 ' + health.apiVersion;
  }
  const order = compareVersions(health.version, web.minBackendVersion);
  if (Number.isNaN(order) || order < 0) {
    return '网页需要 weflow ' + web.minBackendVersion + ' 或更新的版本，当前是 ' + health.version;
  }
  return null;
}

export async function checkBackend(signal?: AbortSignal): Promise<Backend> {
  try {
    const response = await fetch('/health', { signal, cache: 'no-store' });
    if (!response.ok) return { state: 'offline' };
    const health = (await response.json()) as Health;
    const reason = incompatibility(health);
    return reason ? { state: 'incompatible', health, reason } : { state: 'ready', health };
  } catch (problem) {
    if (problem instanceof DOMException && problem.name === 'AbortError') throw problem;
    return { state: 'offline' };
  }
}

const RECHECK_MS = 15_000;

/** The backend's state, checked on start, every 15 s and when the tab comes back; `recheck` checks now. */
export function useBackend(): [Backend, () => void] {
  const [backend, setBackend] = useState<Backend>({ state: 'checking' });
  const [round, setRound] = useState(0);
  const recheck = useCallback(() => setRound(n => n + 1), []);

  useEffect(() => {
    const controller = new AbortController();
    checkBackend(controller.signal).then(setBackend, () => {});
    const timer = window.setTimeout(recheck, RECHECK_MS);
    return () => {
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [round, recheck]);

  useEffect(() => {
    const visible = () => {
      if (document.visibilityState === 'visible') recheck();
    };
    document.addEventListener('visibilitychange', visible);
    return () => document.removeEventListener('visibilitychange', visible);
  }, [recheck]);

  return [backend, recheck];
}
