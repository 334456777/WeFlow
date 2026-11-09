// The WebUI versions (#406), through `weflow web`: those kept on the server's machine and those published. A version
// picked through the server's own API is shown at once when the server follows the picked one (the wiki's WebUI page).
import { call, get } from './client';
import type * as Contract from './contract.gen';

export type KeptVersion = Contract.WebInstalled;
export type PublishedVersion = Contract.WebPublished;

export const listKept = (signal?: AbortSignal) => get<KeptVersion[]>('web/list', {}, signal);

/** Asks the registry, so it takes a few seconds and fails offline. */
export const listPublished = (signal?: AbortSignal) => get<PublishedVersion[]>('web/list', { remote: true }, signal);

/** `web use`: downloads the version when needed and picks it. */
export const pickVersion = (version: string) => call<Contract.WebChosen>('PUT', 'web/use', { version });

/** `web prune`: removes the versions neither picked nor the default one of this weflow. */
export const pruneVersions = () => call<Contract.WebPruned>('DELETE', 'web/prune', {});

/** The version the server shows at `/ui/` now, from its `weflow-web.json`; null when it has none (the dev server). */
export async function shownVersion(): Promise<string | null> {
  try {
    const response = await fetch('/ui/weflow-web.json', { cache: 'no-store' });
    if (!response.ok) return null;
    const meta: unknown = await response.json();
    const version = meta && typeof meta === 'object' ? (meta as { version?: unknown }).version : undefined;
    return typeof version === 'string' ? version : null;
  } catch {
    return null;
  }
}
