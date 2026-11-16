// Whether this page may hold its long-lived connections (#799). Over HTTP/1.1 Chrome opens six connections at most to
// a server, and all the tabs of the browser share them; every page keeps an event stream open (backend.ts, and
// useJobs.ts while a job runs). A page that cannot be seen lets go of its streams, so that those in view always have
// room, and takes them up again the moment it is seen.
//
// - hidden (another tab, a minimized or covered window): away after `HIDDEN_MS`; in view again: back at once
// - `pagehide` (into the back/forward cache, or gone) and `freeze`: away at once; `pageshow` and `resume`: as the
//   page's visibility says

/** How long a hidden page keeps its streams, so that a glance at another tab and back connects nothing again. */
export const HIDDEN_MS = 10_000;

let here = true;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<(here: boolean) => void>();

function set(next: boolean): void {
  if (next === here) return;
  here = next;
  for (const listener of [...listeners]) listener(next);
}

const judge = () => {
  clearTimeout(timer);
  timer = undefined;
  if (document.visibilityState === 'visible') set(true);
  else if (here) timer = setTimeout(() => set(false), HIDDEN_MS);
};

const leave = () => {
  clearTimeout(timer);
  timer = undefined;
  set(false);
};

function watch(): void {
  document.addEventListener('visibilitychange', judge);
  document.addEventListener('freeze', leave);
  document.addEventListener('resume', judge);
  window.addEventListener('pagehide', leave);
  window.addEventListener('pageshow', judge);
  // a page opened in the background counts as hidden from now
  here = true;
  judge();
}

function unwatch(): void {
  document.removeEventListener('visibilitychange', judge);
  document.removeEventListener('freeze', leave);
  document.removeEventListener('resume', judge);
  window.removeEventListener('pagehide', leave);
  window.removeEventListener('pageshow', judge);
  clearTimeout(timer);
  timer = undefined;
  here = true;
}

/** Whether the page may hold its streams now. */
export function present(): boolean {
  return here;
}

/** Calls `listener` each time the page goes away or comes back; returns how to stop. */
export function onPresence(listener: (here: boolean) => void): () => void {
  if (!listeners.size) watch();
  listeners.add(listener);
  return () => {
    if (listeners.delete(listener) && !listeners.size) unwatch();
  };
}
