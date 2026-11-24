// The profiles of `config profiles` (#216) while the server is there: read again for another server or account
// (`scope`), when the setup saved a setting, and when a job that saves a key has succeeded.
import { useCallback, useEffect, useRef, useState } from 'react';
import { isFinished, type Job } from '../api/jobs';
import { getProfiles, savedKey, type Profile } from '../api/setup';

/** The profiles as read, and what reads them again. */
export interface ProfilesState {
  /** null until read, and when it could not be. */
  profiles: Profile[] | null;
  /** The server could not say: the read failed, or did not answer a list. */
  failed: boolean;
  reload(): void;
}

/**
 * `config profiles` while `enabled`, read again for another `scope`, on `reload`, and once one of `jobs` that saves a
 * key (`key db --save`, `key image --save`) has succeeded.
 */
export function useProfiles(enabled: boolean, scope: number, jobs: readonly Job[]): ProfilesState {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  // here rather than in the setup, which is closed or on another step while the job runs (#1138); a job first seen
  // ended changed nothing since the profiles were read
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

/** Whether the person closed the setup this tab opened by itself; it then waits to be opened from the settings. */
export function setupDismissed(): boolean {
  try { return sessionStorage.getItem(DISMISSED_KEY) === 'dismissed'; } catch { return false; }
}

/** Remembers, for this tab, that the setup was closed. */
export function dismissSetup(): void {
  try { sessionStorage.setItem(DISMISSED_KEY, 'dismissed'); } catch { /* asked again after a reload */ }
}
