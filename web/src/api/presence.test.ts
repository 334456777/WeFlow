import { afterEach, beforeEach, describe, expect, it, vi } from '#test';
import { HIDDEN_MS, onPresence, present } from './presence';

/** 把页面设为可见或隐藏；`announce` 时像浏览器那样发出事件。 */
function setVisibility(state: 'visible' | 'hidden', announce = true): void {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  if (announce) document.dispatchEvent(new Event('visibilitychange'));
}

/** 页面生命周期的 `pagehide` 或 `pageshow` 事件，`persisted` 表示是否进出往返缓存。 */
const lifecycle = (type: 'pagehide' | 'pageshow', persisted: boolean) => Object.assign(new Event(type), { persisted });

/** 监听页面的离开和回来：收到的通知，以及停止监听的函数。 */
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
    // 切到别的标签页看一眼，什么都不变
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
    // 没有留下会再通知一次的计时器
    vi.advanceTimersByTime(HIDDEN_MS);
    expect(p.heard).toEqual([false]);
    setVisibility('visible', false);
    window.dispatchEvent(lifecycle('pageshow', true));
    expect(p.heard).toEqual([false, true]);
    // 页面的第一个 pageshow 什么都不变
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
    // 没有监听者时不监视任何事件，下一个监听者从头开始
    expect(present()).toBe(true);
    setVisibility('visible');
    expect(present()).toBe(true);
  });
});
