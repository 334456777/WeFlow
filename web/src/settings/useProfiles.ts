// 有服务端时读取的 `config profiles` 档案（#216）：换了服务端或账号（`scope`）、配置向导保存了设置、保存密钥的
// 作业成功后都重新读取。
import { useCallback, useEffect, useRef, useState } from 'react';
import { isFinished, type Job } from '../api/jobs';
import { getProfiles, savedKey, type Profile } from '../api/setup';

/** 读到的档案，以及重新读取的函数。 */
export interface ProfilesState {
  /** 读到之前和读取失败时为 null。 */
  profiles: Profile[] | null;
  /** 服务端没能给出档案：读取失败，或者回答的不是列表。 */
  failed: boolean;
  reload(): void;
}

/**
 * `enabled` 时读取 `config profiles`；`scope` 变化、调用 `reload`，以及 `jobs` 中保存密钥的作业（`key db --save`、
 * `key image --save`）成功后都重新读取。
 */
export function useProfiles(enabled: boolean, scope: number, jobs: readonly Job[]): ProfilesState {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  // 放在这里而不是配置向导里，因为作业运行期间向导可能已经关闭或在另一步（#1138）；第一次看到时就已经结束的作业，
  // 在这次读取档案之前就结束了，不必重读
  const saving = useRef(new Set<string>());
  useEffect(() => {
    let saved = false;
    for (const job of jobs) {
      if (!savedKey(job)) continue;
      if (!isFinished(job.status)) saving.current.add(job.id);
      else if (saving.current.delete(job.id) && job.status === 'succeeded') saved = true;
    }
    if (saved) reload();
  }, [jobs, reload]);
  useEffect(() => {
    if (!enabled) { setProfiles(null); setFailed(false); return; }
    const controller = new AbortController();
    getProfiles(controller.signal).then(found => {
      setProfiles(Array.isArray(found) ? found : null);
      setFailed(!Array.isArray(found));
    }, () => { if (!controller.signal.aborted) { setProfiles(null); setFailed(true); } });
    return () => controller.abort();
  }, [enabled, scope, round]);
  return { profiles, failed, reload };
}

const DISMISSED_KEY = 'weflow.setup';

/** 用户是否关掉了这个标签页自动打开的配置向导；关掉后要从设置里再打开。 */
export function setupDismissed(): boolean {
  try { return sessionStorage.getItem(DISMISSED_KEY) === 'dismissed'; } catch { return false; }
}

/** 在这个标签页里记住配置向导已被关掉。 */
export function dismissSetup(): void {
  try { sessionStorage.setItem(DISMISSED_KEY, 'dismissed'); } catch { /* 记不住时，刷新后会再打开一次 */ }
}
