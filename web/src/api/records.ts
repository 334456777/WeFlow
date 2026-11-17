// Whose records the page shows, from the connection's state alone (#876, #879). A module of its own, which loads nothing
// of the connection's, so that what needs only this (scope.ts, which the tests' preload loads before a DOM is there)
// does not load it.
import type { Backend } from './backend';

/**
 * Whether the page shows the server's records: while it is ready, and while it is gone after it was ready, with what
 * was read before it went.
 */
export const holdsRecords = (backend: Backend): boolean =>
  backend.state === 'ready' || (backend.state === 'offline' && backend.held === true);

/**
 * Whose records the page shows: the server's (`holdsRecords`); the sample's (src/mock, #879) while no server has
 * answered or the one there does not suit this page; none while the server is looked for or refuses this tab's token.
 */
export function recordsOf(backend: Backend): 'server' | 'sample' | null {
  if (holdsRecords(backend)) return 'server';
  return backend.state === 'offline' || backend.state === 'incompatible' ? 'sample' : null;
}
