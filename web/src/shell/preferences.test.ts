import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import { keptMedia, usePreferences } from './preferences';

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.documentElement.classList.remove('reduced-motion');
});

describe('the preferences, kept in this browser (#702)', () => {
  it('start from the defaults: live messages, no media, full motion', () => {
    const { result } = renderHook(() => usePreferences());
    expect(result.current.realtime).toBe(true);
    expect(result.current.media).toEqual([]);
    expect(result.current.reducedMotion).toBe(false);
  });

  it('come back after a reload', () => {
    const first = renderHook(() => usePreferences());
    act(() => { first.result.current.setMedia(['image', 'file']); first.result.current.setReducedMotion(true); });
    expect(document.documentElement.classList.contains('reduced-motion')).toBe(true);
    first.unmount();
    const { result } = renderHook(() => usePreferences());
    expect(result.current.media).toEqual(['image', 'file']);
    expect(result.current.reducedMotion).toBe(true);
  });

  it('keep only the media this page knows, in its order', () => {
    expect(keptMedia('["file","avatar","image"]')).toEqual(['image', 'file']);
    expect(keptMedia('not json')).toEqual([]);
    expect(keptMedia('{"image":true}')).toEqual([]);
    expect(keptMedia(null)).toEqual([]);
  });
});
