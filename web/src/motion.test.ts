import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { contentIn, durationMs, pageIn, timing, usePresence } from './motion';
import { type PageTurns, usePageTurn } from './pageTurn';

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

describe('pageIn', () => {
  it('turns the page and brings elements in as content does, or by the skin’s own animation; neither with reduced motion (#832)', () => {
    const element = document.createElement('div');
    const animate = vi.fn();
    element.animate = animate as unknown as Element['animate'];
    const root = document.documentElement.style;
    root.setProperty('--duration-base', '200ms');
    const turned = vi.fn();
    const { unmount } = renderHook(() => { turned(usePageTurn()); });
    const turns = () => turned.mock.lastCall![0] as PageTurns;
    const start = turns();
    // so many turns and pages since the start
    const since = (turn: number, page: number) => ({ turn: start.turn + turn, page: start.page + page });

    act(() => pageIn([element]));
    expect(animate).toHaveBeenCalledTimes(1);
    expect(turns()).toEqual(since(1, 1));

    // the skin's animation, played from its start, and given back to the style sheets when it ends
    root.setProperty('--page-in', 'paper-in 200ms steps(3)');
    act(() => pageIn([element]));
    expect(animate).toHaveBeenCalledTimes(1);
    expect(element.style.animation).toBe('paper-in 200ms steps(3)');
    expect(turns()).toEqual(since(2, 2));
    const inner = element.appendChild(document.createElement('span'));
    inner.dispatchEvent(new Event('animationend', { bubbles: true }));
    expect(element.style.animation).toBe('paper-in 200ms steps(3)');
    element.dispatchEvent(new Event('animationend'));
    expect(element.style.animation).toBe('');

    // content that comes in without turning a page
    act(() => pageIn([element], { turn: false }));
    expect(element.style.animation).toBe('paper-in 200ms steps(3)');
    expect(turns()).toEqual(since(2, 2));

    // another item's detail in the same page: a turn, but no other page
    act(() => pageIn([element], { detail: true }));
    expect(turns()).toEqual(since(3, 2));

    // another page before it ends: the browser cancels it to start it again and says so once it runs again, which
    // stops nothing; cancelled for good (the element hidden), it is given back
    act(() => pageIn([element]));
    const cancel = () => element.dispatchEvent(Object.assign(new Event('animationcancel'), { animationName: 'paper-in' }));
    const runs = (playState: string) => { element.getAnimations = () => [{ animationName: 'paper-in', playState }] as unknown as Animation[]; };
    runs('running');
    cancel();
    expect(element.style.animation).toBe('paper-in 200ms steps(3)');
    runs('idle');
    cancel();
    expect(element.style.animation).toBe('');

    root.setProperty('--duration-base', '0ms');
    act(() => pageIn([element]));
    expect(element.style.animation).toBe('');
    expect(turns()).toEqual(since(4, 3));
    unmount();
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
