// The account setup of `weflow serve` (#216): the profiles and how far each is set up, finding the WeChat data
// directory and its accounts, saving settings, and testing that the database opens. The keys are fetched by
// `key db --save` / `key image --save`, which store them on the server: they never reach the page.
import { call, get } from './client';

/** A row of `config profiles`: the keys only as whether they are set. */
export interface Profile {
  name: string;
  current: boolean;
  wxid: string | null;
  dbPath: string | null;
  decryptKey: boolean;
  imageKeys: boolean;
  /** Enough to open the database: a data directory and a database key. */
  complete: boolean;
}

export interface Account {
  wxid: string;
  path: string;
}

export const getProfiles = (signal?: AbortSignal) => get<Profile[]>('config/profiles', {}, signal);

/** The current profile, or undefined when the config names one it does not have. */
export const currentProfile = (profiles: readonly Profile[]) => profiles.find(p => p.current);

/** `db detect`: the usual WeChat data directories that exist on the server's computer. */
export async function detectDirs(signal?: AbortSignal): Promise<string[]> {
  const found = await get<{ candidates?: { path: string; exists: boolean }[] }>('db/detect', {}, signal);
  return (found.candidates ?? []).filter(c => c.exists).map(c => c.path);
}

/** `db wxid <root>`: the accounts of a data directory, by the wxid `config set wxid` takes. */
export async function findAccounts(root: string, signal?: AbortSignal): Promise<Account[]> {
  const found = await get<{ accounts?: Account[] }>('db/wxid', { root }, signal);
  return found.accounts ?? [];
}

/** `config set <key> <value>` for the current profile. */
export const setConfig = (key: string, value: string) => call<unknown>('PUT', 'config/set', { key, value });

/** `db test`: opens the configured database with its key. */
export const testDatabase = (signal?: AbortSignal) => get<{ connected?: boolean }>('db/test', {}, signal);

/** `key image --save` (Windows): the image keys from WeChat's cache, saved on the server. */
export const saveImageKeys = () => call<{ saved?: string[]; verified?: boolean }>('POST', 'key/image', { save: true });

/** `runtime info`: the server's target, to tell which ways of getting keys it has. */
export async function serverPlatform(signal?: AbortSignal): Promise<'windows' | 'macos' | 'linux' | 'other'> {
  const info = await get<{ target?: string }>('runtime/info', {}, signal);
  const target = info.target ?? '';
  return target.includes('windows') ? 'windows' : target.includes('apple') ? 'macos' : target.includes('linux') ? 'linux' : 'other';
}

/** A database key as `config set decrypt_key` takes it: 64 hexadecimal digits. */
export const isDatabaseKey = (value: string) => /^[0-9a-fA-F]{64}$/.test(value.trim());
