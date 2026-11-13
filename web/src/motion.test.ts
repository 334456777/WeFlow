import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { contentIn, durationMs, installSprings, springCss, springTiming, springs, usePresence } from './motion';
import type { SpringName } from './motion';

afterEach(() => {
  vi.useRealTimers();
  document.documentElement.removeAttribute('style');
  document.getElementById('weflow-springs')?.remove();
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

  it('reads the duration a spring token starts with', () => {
    document.documentElement.style.setProperty('--spring-settle', '500ms linear(0, 0.5, 1)');
    expect(durationMs('--spring-settle')).toBe(500);
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

describe('springs', () => {
  const names = Object.keys(springs) as SpringName[];

  it('trace each spring as a duration and a linear() curve that ends at rest', () => {
    for (const name of names) {
      const css = springCss(name);
      expect(css).toMatch(/^\d+ms linear\(0, .*1\)$/);
      expect(Number.parseInt(css, 10)).toBeGreaterThan(0);
    }
  });

  it('overshoot only where they are meant to', () => {
    const peak = (name: SpringName) => Math.max(...springCss(name).slice(springCss(name).indexOf('(') + 1, -1).split(',').map(Number));
    expect(peak('settle')).toBeLessThanOrEqual(1);
    expect(peak('feedback')).toBeGreaterThan(peak('control'));
    expect(peak('control')).toBeGreaterThan(1);
  });

  it('are written as tokens in a stylesheet ahead of every other one', () => {
    const other = document.createElement('style');
    document.head.append(other);
    installSprings();
    installSprings();
    const style = document.getElementById('weflow-springs')!;
    expect(document.head.firstElementChild).toBe(style);
    expect(document.querySelectorAll('#weflow-springs')).toHaveLength(1);
    for (const name of names) {
      expect(style.textContent).toContain('--spring-' + name + ': ');
      expect(style.textContent).toContain('--spring-' + name + '-duration: ' + String(Number.parseInt(springCss(name), 10)) + 'ms;');
    }
    other.remove();
  });

  it('give the Web Animations API a duration and an easing, and nothing to run with reduced motion', () => {
    document.documentElement.style.setProperty('--spring-settle', '500ms linear(0, 0.5, 1)');
    expect(springTiming('settle')).toEqual({ duration: 500, easing: 'linear(0, 0.5, 1)' });
    document.documentElement.style.setProperty('--spring-settle', '0ms');
    expect(springTiming('settle').duration).toBe(0);
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
