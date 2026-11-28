// WebUI 的版本（#406），经由 `weflow web` 读写：服务端所在机器上保存的版本和已发布的版本。
// 通过服务端自己的 API 选定的版本，在服务端跟随选定的版本时立即显示（见 Wiki 的「WebUI」页）。
import { call, get } from './client';
import type * as Contract from './contract.gen';

/** 服务端所在机器上保存的一个版本（`web list`）。 */
export type KeptVersion = Contract.WebInstalled;
/** 一个已发布的版本（`web list --remote`）。 */
export type PublishedVersion = Contract.WebPublished;

/** `web list`：服务端所在机器上保存的版本。 */
export const listKept = (signal?: AbortSignal) => get<KeptVersion[]>('web/list', {}, signal);

/** `web list --remote`：已发布的版本。要查询发布源，需要几秒钟，离线时失败。 */
export const listPublished = (signal?: AbortSignal) => get<PublishedVersion[]>('web/list', { remote: true }, signal);

/** `web use`：需要时下载这个版本，并选定它。 */
export const pickVersion = (version: string) => call<Contract.WebChosen>('PUT', 'web/use', { version });

/** `web prune`：删除既没有选定、也不是这个 weflow 默认版本的版本。 */
export const pruneVersions = () => call<Contract.WebPruned>('DELETE', 'web/prune', {});

/** 服务端此刻在 `/ui/` 显示的版本，取自它的 `weflow-web.json`；没有这个文件（开发服务器）时为 null。 */
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
