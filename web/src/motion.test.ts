import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { contentIn, durationMs, timing, usePresence } from './motion';

afterEach(() => {
  vi.useRealTimers();
  document.documentElement.removeAttribute('style');
});

describe('durationMs', () => {
  it('reads milliseconds and seconds from a token', () => {
    document.documentElement.style.setProperty('--duration-exit', '150ms');
    document.documentElement.style.setProperty('--duration-page', '.45s');
    expect(durationMs('--duration-exit')).toBe(150);
    expect(durationMs('--duration-page')).toBe(450);
  });

  it('treats a missing token as no wait', () => {
    expect(durationMs('--duration-missing')).toBe(0);
  });
});

describe('contentIn', () => {
  it('brings elements in as content-in does, from the tokens, and does nothing when they are 0 (#776)', () => {
    const element = document.createElement('div');
    const animate = vi.fn();
    element.animate = animate as unknown as Element['animate'];
    const root = document.documentElement.style;
    root.setProperty('--duration-base', '200ms');
    root.setProperty('--enter-offset', '6px');
    root.setProperty('--ease-out', 'cubic-bezier(.165, .84, .44, 1)');
    contentIn([element]);
    expect(animate).toHaveBeenCalledWith(
      [{ opacity: 0, transform: 'translateY(calc(.5 * 6px))' }, { opacity: 1, transform: 'none' }],
      { duration: 200, easing: 'cubic-bezier(.165, .84, .44, 1)' },
    );
    // reduced motion sets the durations to 0
    root.setProperty('--duration-base', '0ms');
    contentIn([element]);
    expect(animate).toHaveBeenCalledTimes(1);
  });
});

describe('timing', () => {
  it('gives the Web Animations API a duration and a curve from their tokens, and nothing to run with reduced motion', () => {
    const root = document.documentElement.style;
    root.setProperty('--duration-panel', '300ms');
    root.setProperty('--ease-snap', 'cubic-bezier(.32, .72, 0, 1)');
    expect(timing('--duration-panel', '--ease-snap')).toEqual({ duration: 300, easing: 'cubic-bezier(.32, .72, 0, 1)' });
    expect(timing('--duration-panel', '--ease-missing').easing).toBe('ease');
    root.setProperty('--duration-panel', '0ms');
    expect(timing('--duration-panel', '--ease-snap').duration).toBe(0);
  });
});

describe('usePresence', () => {
  it('keeps a cleared value for the exit animation, then drops it', () => {
    vi.useFakeTimers();
    document.documentElement.style.setProperty('--duration-exit', '150ms');
    const { result, rerender } = renderHook(({ value }: { value: string }) => usePresence(value), { initialProps: { value: '' } });
    expect(result.current).toBeNull();

    rerender({ value: '已复制' });
    expect(result.current).toEqual({ value: '已复制', closing: false });

    rerender({ value: '' });
    expect(result.current).toEqual({ value: '已复制', closing: true });
    act(() => { vi.advanceTimersByTime(149); });
    expect(result.current).not.toBeNull();
    act(() => { vi.advanceTimersByTime(1); });
    expect(result.current).toBeNull();
  });

  it('comes back at once when the value returns during the exit', () => {
    vi.useFakeTimers();
    document.documentElement.style.setProperty('--duration-exit', '150ms');
    const { result, rerender } = renderHook(({ open }: { open: boolean }) => usePresence(open), { initialProps: { open: true } });
    rerender({ open: false });
    expect(result.current?.closing).toBe(true);
    rerender({ open: true });
    expect(result.current).toEqual({ value: true, closing: false });
    act(() => { vi.advanceTimersByTime(500); });
    expect(result.current).toEqual({ value: true, closing: false });
  });
});
