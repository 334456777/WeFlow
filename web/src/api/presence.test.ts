import { afterEach, beforeEach, describe, expect, it, vi } from '#test';
import { HIDDEN_MS, onPresence, present } from './presence';

/** Makes the page visible or hidden, and says so as the browser does when `announce`. */
function setVisibility(state: 'visible' | 'hidden', announce = true): void {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  if (announce) document.dispatchEvent(new Event('visibilitychange'));
}

/** What the page's lifecycle says in `persisted` (the back/forward cache) for `pagehide` and `pageshow`. */
const lifecycle = (type: 'pagehide' | 'pageshow', persisted: boolean) => Object.assign(new Event(type), { persisted });

/** Listens to the presence: what it heard, and how to stop. */
function listen() {
  const heard: boolean[] = [];
  const off = onPresence(here => heard.push(here));
  return { heard, off };
}

describe('presence (#799)', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => {
    vi.useRealTimers();
    delete (document as { visibilityState?: unknown }).visibilityState;
  });

  it('lets go after the page has been hidden a while, and is back the moment it is seen', () => {
    const p = listen();
    expect(present()).toBe(true);
    // a glance at another tab changes nothing
    setVisibility('hidden');
    vi.advanceTimersByTime(HIDDEN_MS - 1);
    setVisibility('visible');
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([]);
    setVisibility('hidden');
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([false]);
    expect(present()).toBe(false);
    setVisibility('visible');
    expect(p.heard).toEqual([false, true]);
    p.off();
  });

  it('lets go at once in the back/forward cache, and is back when the page is shown again', () => {
    const p = listen();
    setVisibility('hidden');
    window.dispatchEvent(lifecycle('pagehide', true));
    expect(p.heard).toEqual([false]);
    // no timer left to say it again
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([false]);
    setVisibility('visible', false);
    window.dispatchEvent(lifecycle('pageshow', true));
    expect(p.heard).toEqual([false, true]);
    // the first pageshow of a page changes nothing
    window.dispatchEvent(lifecycle('pageshow', false));
    expect(p.heard).toEqual([false, true]);
    p.off();
  });

  it('lets go when the page is frozen, and stays away when it resumes out of view', () => {
    const p = listen();
    setVisibility('hidden', false);
    document.dispatchEvent(new Event('freeze'));
    expect(p.heard).toEqual([false]);
    document.dispatchEvent(new Event('resume'));
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([false]);
    setVisibility('visible');
    expect(p.heard).toEqual([false, true]);
    p.off();
  });

  it('counts a page opened in the background as hidden from then', () => {
    setVisibility('hidden', false);
    const p = listen();
    expect(present()).toBe(true);
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([false]);
    p.off();
    // no one listening: nothing is watched, and the next listener starts over
    expect(present()).toBe(true);
    setVisibility('visible');
    expect(present()).toBe(true);
  });
});
