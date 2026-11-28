// `weflow serve` 的账号设置（#216）：档案及其设置进度，查找微信的数据目录和其中的账号，保存设置，
// 以及测试能否打开数据库。密钥由 `key db --save` / `key image --save` 获取并保存在服务端，不会传到网页。
import { call, get } from './client';
import { commandName, type Job } from './jobs';
import { t } from '../i18n';
import type * as Contract from './contract.gen';

/** `config profiles` 的一行：密钥只给出有没有设置（契约的类型，#430）。 */
export type Profile = Contract.ProfileSummary;

/** 微信数据目录里的一个账号（`db wxid`）。 */
export type Account = Contract.DbAccount;

/** `config profiles`：全部档案，在服务进程内读取。 */
export const getProfiles = (signal?: AbortSignal) => get<Profile[]>('api/config/profiles', {}, signal);

/** 当前的档案；配置里指定的档案不存在时为 undefined。 */
export const currentProfile = (profiles: readonly Profile[]) => profiles.find(p => p.current);

/** `db detect`：服务端所在电脑上存在的常见微信数据目录。 */
export async function detectDirs(signal?: AbortSignal): Promise<string[]> {
  const found = await get<Partial<Contract.DbCandidates>>('api/db/detect', {}, signal);
  return (found.candidates ?? []).filter(c => c.exists).map(c => c.path);
}

/** `db wxid <root>`：数据目录里的账号，以 `config set wxid` 接受的 wxid 表示。 */
export async function findAccounts(root: string, signal?: AbortSignal): Promise<Account[]> {
  const found = await get<Partial<Contract.DbAccounts>>('api/db/wxid', { root }, signal);
  return found.accounts ?? [];
}

/** 对当前档案执行 `config set <key> <value>`。 */
export const setConfig = (key: string, value: string) => call<Contract.ConfigWritten>('PUT', 'config/set', { key, value });

/** `db test`：用配置的密钥打开配置的数据库。 */
export const testDatabase = (signal?: AbortSignal) => get<Partial<Contract.DbConnection>>('api/db/test', {}, signal);

/** `key image --save`（Windows、Linux 和 macOS）：从微信的缓存取得图片密钥，保存在服务端。 */
export const saveImageKeys = () => call<Contract.ImageKeys>('POST', 'key/image', { save: true });

/** 作业保存到当前档案里的密钥：`key db --save` 为 `db`，`key image --save` 为 `image`，其他作业为 null。 */
export function savedKey(job: Pick<Job, 'command'>): 'db' | 'image' | null {
  if (!job.command.includes('--save')) return null;
  const name = commandName(job.command);
  return name === 'key db' ? 'db' : name === 'key image' ? 'image' : null;
}

/** 是否是 `config set decrypt_key` 接受的数据库密钥：64 位十六进制数字。 */
export const isDatabaseKey = (value: string) => /^[0-9a-fA-F]{64}$/.test(value.trim());

/** `config set current_profile <name>`：切换到这个档案，档案不存在时新建。 */
export const switchProfile = (name: string) => setConfig('current_profile', name);

/** `cache clear-account` 删除的东西：WeFlow 为这个账号保存的文件，以及从档案里去掉的密钥。 */
export type ClearPlan = Contract.RemovalPlan;

/** `cache clear-account --dry-run`：清除当前账号会删除什么，不实际删除。 */
export const planClearAccount = () => call<ClearPlan>('DELETE', 'cache/clear-account', { 'dry-run': true });

/** `cache clear-account --yes`：删除当前账号的这些文件和密钥。 */
export const clearAccount = () => call<Contract.AccountCleared>('DELETE', 'cache/clear-account', { yes: true });

/**
 * 新档案名的问题，没有问题时为 null：`config set current_profile` 接受的新档案名只含文字、数字、`-` 和 `_`，
 * 且不能与已有的档案重名。
 */
export function profileNameProblem(name: string, profiles: readonly Profile[]): string | null {
  const typed = name.trim();
  if (!typed) return t('请填写档案名称。');
  if (!/^[\p{L}\p{N}_-]+$/u.test(typed)) return t('档案名称只能包含文字、数字、- 和 _。');
  if (profiles.some(p => p.name === typed)) return t('已经有这个档案了。');
  return null;
}
