// The profiles of `config profiles` (#216) while the server is there: read again for another server or account
// (`scope`), and when the setup saved a setting.
import { useCallback, useEffect, useState } from 'react';
import { getProfiles, type Profile } from '../api/setup';

export interface ProfilesState {
  /** null until read, and when it could not be. */
  profiles: Profile[] | null;
  /** The server could not say: a `weflow` older than `config profiles`, or a failure. */
  failed: boolean;
  reload(): void;
}

export function useProfiles(enabled: boolean, scope: number): ProfilesState {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
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

export function dismissSetup(): void {
  try { sessionStorage.setItem(DISMISSED_KEY, 'dismissed'); } catch { /* asked again after a reload */ }
}
