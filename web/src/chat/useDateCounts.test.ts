import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from '#test';
import { emitLive } from '../api/live';
import { useDateCounts } from './useDateCounts';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });

it('reads the days again when a calendar opens after a message came (#201)', async () => {
  let counts: Record<string, number> = { '2026-10-01': 2 };
  const calls: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => { calls.push(url); return json({ sessionId: 'wxid_a', counts }); }));
  const { result } = renderHook(() => useDateCounts('wxid_a'));
  expect(result.current.counts).toBeNull();
  act(() => result.current.request());
  await waitFor(() => expect(result.current.counts).toEqual({ '2026-10-01': 2 }));

  // 再次打开，或者别的会话来了消息：用缓存
  act(() => result.current.request());
  act(() => emitLive({ type: 'message.new', sessionId: 'wxid_b' }));
  act(() => result.current.request());
  await act(async () => {});
  expect(calls).toHaveLength(1);

  // 这个会话来了消息：下次打开日历时重新读取，这期间显示之前读到的日子
  counts = { '2026-10-01': 2, '2026-10-02': 1 };
  act(() => emitLive({ type: 'message.new', sessionId: 'wxid_a' }));
  expect(result.current.counts).toEqual({ '2026-10-01': 2 });
  act(() => result.current.request());
  expect(result.current.counts).toEqual({ '2026-10-01': 2 });
  await waitFor(() => expect(result.current.counts).toEqual({ '2026-10-01': 2, '2026-10-02': 1 }));
  expect(calls).toHaveLength(2);
});
