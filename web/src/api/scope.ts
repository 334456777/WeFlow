// Whose records the page holds (#198). What the page keeps between views (messages, group members, the days with
// messages) belongs to one server and one account; once either may have changed, a new scope starts: everything
// kept is forgotten, and an answer to a request of the old scope is not kept.
//
// A new scope starts when the server's `data` (in `/health`: it changes with each start of `weflow serve` and with
// its configuration, so with the account) is not the one seen before, and when the server comes back after it was
// gone (an older server says no `data`).
import { useEffect, useRef, useState } from 'react';
import type { Backend } from './backend';

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
  const seen = useRef<{ data: string | null; lost: boolean } | null>(null);
  const [current, setCurrent] = useState(currentScope);
  useEffect(() => {
    if (backend.state === 'checking') return;
    if (backend.state !== 'ready') {
      if (seen.current) seen.current.lost = true;
      return;
    }
    const data = backend.health.data ?? null;
    const last = seen.current;
    seen.current = { data, lost: false };
    if (last && (last.lost || last.data !== data)) {
      newScope();
      setCurrent(currentScope());
    }
  }, [backend]);
  return current;
}
