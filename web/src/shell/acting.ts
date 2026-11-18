// The server the page acts on (#896): the one that answers, or the sample server in its place, which runs jobs and
// keeps settings in the page as a server would. What changes something or follows it (jobs, the settings) uses it.
import { useEffect, useState } from 'react';
import type { Backend, Health } from '../api/backend';
import { get } from '../api/client';

/**
 * What `/health` says of the server the page acts on: the server's while it is ready, the sample server's while it
 * answers (read once, as a server's is once it has started); null otherwise.
 */
export function useActingHealth(backend: Backend, sample: boolean): Health | null {
  const [sampled, setSampled] = useState<Health | null>(null);
  useEffect(() => {
    if (!sample) { setSampled(null); return; }
    let live = true;
    get<Health>('health').then(health => { if (live) setSampled(health); }, () => { /* the settings wait for it */ });
    return () => { live = false; };
  }, [sample]);
  return backend.state === 'ready' ? backend.health : sample ? sampled : null;
}
