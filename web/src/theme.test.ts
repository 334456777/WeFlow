import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from '#test';
import { savedTheme, THEME_KEY, themeOf, useTheme } from './theme';

/** A system in light or dark mode, which can switch while the page is open. */
function system(dark: boolean) {
  const listeners = new Set<() => void>();
  const query = {
    get matches() { return dark; },
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
  };
  vi.stubGlobal('matchMedia', vi.fn(() => query));
  return { switchTo(next: boolean) { dark = next; for (const listener of [...listeners]) listener(); }, listeners };
}

beforeEach(() => { localStorage.clear(); delete document.documentElement.dataset.theme; });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe('the colour theme (#252)', () => {
  it('follows the system by default, also when it switches', () => {
    const os = system(true);
    expect(savedTheme()).toBe('system');
    const { result } = renderHook(() => useTheme());
    expect(result.current[0]).toBe('system');
    expect(document.documentElement.dataset.theme).toBe('dark');
    act(() => os.switchTo(false));
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('keeps a chosen theme whatever the system does, and forgets it on going back to the system', () => {
    const os = system(false);
    const { result } = renderHook(() => useTheme());
    act(() => result.current[1]('dark'));
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(localStorage.getItem(THEME_KEY)).toBe('dark');
    // a chosen theme stops listening to the system
    expect(os.listeners.size).toBe(0);
    act(() => os.switchTo(true));
    act(() => result.current[1]('light'));
    expect(document.documentElement.dataset.theme).toBe('light');

    act(() => result.current[1]('system'));
    expect(localStorage.getItem(THEME_KEY)).toBeNull();
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('reads back what was kept, and anything else as the system', () => {
    system(false);
    localStorage.setItem(THEME_KEY, 'dark');
    expect(savedTheme()).toBe('dark');
    localStorage.setItem(THEME_KEY, 'sepia');
    expect(savedTheme()).toBe('system');
    expect(themeOf('system')).toBe('light');
  });
});
