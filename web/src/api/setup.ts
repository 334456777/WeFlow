// The account setup of `weflow serve` (#216): the profiles and how far each is set up, finding the WeChat data
// directory and its accounts, saving settings, and testing that the database opens. The keys are fetched by
// `key db --save` / `key image --save`, which store them on the server: they never reach the page.
import { call, get } from './client';
import type * as Contract from './contract.gen';

/** A row of `config profiles`: the keys only as whether they are set (the contract's, #430). */
export type Profile = Contract.ProfileSummary;

export type Account = Contract.DbAccount;

export const getProfiles = (signal?: AbortSignal) => get<Profile[]>('api/config/profiles', {}, signal);

/** The current profile, or undefined when the config names one it does not have. */
export const currentProfile = (profiles: readonly Profile[]) => profiles.find(p => p.current);

/** `db detect`: the usual WeChat data directories that exist on the server's computer. */
export async function detectDirs(signal?: AbortSignal): Promise<string[]> {
  const found = await get<Partial<Contract.DbCandidates>>('api/db/detect', {}, signal);
  return (found.candidates ?? []).filter(c => c.exists).map(c => c.path);
}

/** `db wxid <root>`: the accounts of a data directory, by the wxid `config set wxid` takes. */
export async function findAccounts(root: string, signal?: AbortSignal): Promise<Account[]> {
  const found = await get<Partial<Contract.DbAccounts>>('api/db/wxid', { root }, signal);
  return found.accounts ?? [];
}

/** `config set <key> <value>` for the current profile. */
export const setConfig = (key: string, value: string) => call<unknown>('PUT', 'config/set', { key, value });

/** `db test`: opens the configured database with its key. */
export const testDatabase = (signal?: AbortSignal) => get<Partial<Contract.DbConnection>>('api/db/test', {}, signal);

/** `key image --save` (Windows): the image keys from WeChat's cache, saved on the server. */
export const saveImageKeys = () => call<{ saved?: string[]; verified?: boolean }>('POST', 'key/image', { save: true });

/** `runtime info`: the server's target, to tell which ways of getting keys it has. */
export async function serverPlatform(signal?: AbortSignal): Promise<'windows' | 'macos' | 'linux' | 'other'> {
  const info = await get<Partial<Contract.RuntimeInfo>>('api/runtime/info', {}, signal);
  const target = info.target ?? '';
  return target.includes('windows') ? 'windows' : target.includes('apple') ? 'macos' : target.includes('linux') ? 'linux' : 'other';
}

/** A database key as `config set decrypt_key` takes it: 64 hexadecimal digits. */
export const isDatabaseKey = (value: string) => /^[0-9a-fA-F]{64}$/.test(value.trim());

/** `config set current_profile <name>`: switches to that profile, made when it is new. */
export const switchProfile = (name: string) => setConfig('current_profile', name);

/** What `cache clear-account` removes: WeFlow's files of the account, and the keys it takes out of the profile. */
export interface ClearPlan {
  paths: { path: string; bytes: number }[];
  bytes: number;
  profileKeys: string[];
}

/** `cache clear-account --dry-run`: what clearing the current account would remove, removing nothing. */
export const planClearAccount = () => call<ClearPlan>('DELETE', 'cache/clear-account', { 'dry-run': true });

/** `cache clear-account --yes`: removes it. */
export const clearAccount = () => call<{ removedPaths?: string[]; freedBytes?: number }>('DELETE', 'cache/clear-account', { yes: true });

/** A name `config set current_profile` takes for a new profile: letters, digits, `-` and `_`, and not one in use. */
export function profileNameProblem(name: string, profiles: readonly Profile[]): string | null {
  const typed = name.trim();
  if (!typed) return '请填写档案名称。';
  if (!/^[\p{L}\p{N}_-]+$/u.test(typed)) return '档案名称只能包含文字、数字、- 和 _。';
  if (profiles.some(p => p.name === typed)) return '已经有这个档案了。';
  return null;
}
