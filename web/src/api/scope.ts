// Whose records the page holds (#198). What the page keeps between views (messages, group members, the days with
// messages) belongs to one server and one account; once either may have changed, a new scope starts: everything
// kept is forgotten, and an answer to a request of the old scope is not kept.
//
// A new scope starts when the server's `data` (in `hello` and each `heartbeat` of `/health/events`: it changes with
// each start of `weflow serve` and with its configuration, so with the account) is not the one seen before, when the
// server comes back after it was gone, and when the sample's records (#879) take the place of a server's.
import { useEffect, useRef, useState } from 'react';
import type { Backend } from './backend';
import { recordsOf } from './records';

let scope = 0;
const forgetters = new Set<() => void>();

/** The scope now: compare it with the one a request started in before keeping its answer. */
export const currentScope = () => scope;

/** Registers what to forget when a new scope starts. */
export function onNewScope(forget: () => void): void {
  forgetters.add(forget);
}

export function newScope(): void {
  scope += 1;
  for (const forget of forgetters) forget();
}

/** The scope for the backend's state as `useBackend` checks it: a new one when the data may have changed. */
export function useScope(backend: Backend): number {
  const seen = useRef<{ data: string | null; lost: boolean; sample: boolean } | null>(null);
  const [current, setCurrent] = useState(currentScope);
  useEffect(() => {
    if (backend.state === 'checking') return;
    const last = seen.current;
    const start = () => { newScope(); setCurrent(currentScope()); };
    if (backend.state !== 'ready') {
      const sample = recordsOf(backend) === 'sample';
      seen.current = { data: last?.data ?? null, lost: true, sample };
      // the sample after a server's records: what was read from the server goes
      if (last && !last.sample && sample) start();
      return;
    }
    const data = backend.health.data ?? null;
    seen.current = { data, lost: false, sample: false };
    // a server after the sample comes as one back after it was gone
    if (last && (last.lost || last.data !== data)) start();
  }, [backend]);
  return current;
}
