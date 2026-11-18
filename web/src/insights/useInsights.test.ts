import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { InsightRecord } from '../api/insights';
import { useInsights, type InsightFilter } from './useInsights';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
const record = (id: string, sessionId: string): InsightRecord =>
  ({ id, createdAt: 0, sessionId, displayName: sessionId, triggerReason: 'activity', insight: id, read: false });
const page = (records: InsightRecord[], total: number, sessionId: string) =>
  ({ records, total, todayCount: 0, unreadCount: total, contacts: [{ sessionId, displayName: sessionId, count: total }] });

describe('the insights read', () => {
  it('leaves out a later page of a filter left: the first page of the next one stays as it came', async () => {
    const A: InsightFilter = { keyword: '', session: 'wxid_a' };
    const B: InsightFilter = { keyword: '', session: 'wxid_b' };
    const firstOfA = Array.from({ length: 50 }, (_, i) => record('a' + i, 'wxid_a'));
    let answerLater: (response: Response) => void = () => {};
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn((url: string) => {
      urls.push(url);
      if (url.includes('offset=50')) return new Promise<Response>(resolve => { answerLater = resolve; });
      return Promise.resolve(url.includes('session=wxid_a') ? json(page(firstOfA, 60, 'wxid_a')) : json(page([record('b0', 'wxid_b')], 1, 'wxid_b')));
    }));
    const { result, rerender } = renderHook(({ filter }) => useInsights(true, 0, filter), { initialProps: { filter: A } });
    await act(async () => {});
    act(() => result.current.more());
    expect(urls.at(-1)).toBe('/insight/records?session=wxid_a&limit=50&offset=50');

    rerender({ filter: B });
    await act(async () => {});
    await act(async () => { answerLater(json(page(Array.from({ length: 10 }, (_, i) => record('a' + (50 + i), 'wxid_a')), 60, 'wxid_a'))); });
    const state = result.current.state;
    if (state.status !== 'ready') throw new Error(state.status);
    expect(state.page.records.map(r => r.id)).toEqual(['b0']);
    expect(state.page.total).toBe(1);
    expect(state.page.contacts.map(c => c.sessionId)).toEqual(['wxid_b']);
    expect(state.loadingMore).toBe(false);
  });

  it('reads the next page of the filter shown, while the first page of another is on its way', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn((url: string) => {
      urls.push(url);
      if (url.includes('keyword=')) return new Promise<Response>(() => {});
      return Promise.resolve(json(page(Array.from({ length: 50 }, (_, i) => record('r' + i, 'wxid_a')), 80, 'wxid_a')));
    }));
    const { result, rerender } = renderHook(({ filter }) => useInsights(true, 0, filter), { initialProps: { filter: { keyword: '', session: null } as InsightFilter } });
    await act(async () => {});
    rerender({ filter: { keyword: '旅行', session: null } });
    act(() => result.current.more());
    expect(urls.at(-1)).toBe('/insight/records?limit=50&offset=50');
  });
});
