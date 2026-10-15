import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { durationMs, usePresence } from './motion';

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
