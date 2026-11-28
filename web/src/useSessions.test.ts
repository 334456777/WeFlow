import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, it, vi } from '#test';
import { emitLive, LIVE_READ_MS } from './api/live';
import { useSessions } from './useSessions';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });

it('reads the list again while live messages keep coming, not only once they stop (#201)', async () => {
  vi.useFakeTimers();
  let reads = 0;
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (!url.startsWith('/api/chat/sessions')) return json({});
    reads++;
    return json([{ username: 'wxid_a', displayName: '林沐', summary: 'm' + reads, sortTimestamp: reads, lastTimestamp: reads, lastMsgType: 1 }]);
  }));
  const { result } = renderHook(() => useSessions(true));
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(result.current[0].status).toBe('ready');
  expect(reads).toBe(1);

  // 每 200 ms 一条消息，间隔比延迟还短：列表仍然跟得上
  const every = 200;
  for (let at = 0; at < 2000; at += every) {
    act(() => emitLive({ type: 'message.new', sessionId: 'wxid_a' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(every); });
  }
  // 每一批的第一条消息之后 500 ms 重读：在 500、1100 和 1700 ms
  expect(LIVE_READ_MS).toBe(500);
  expect(reads).toBe(4);
  const state = result.current[0];
  expect(state.status === 'ready' && state.sessions[0].summary).toBe('m4');

  // 为最后一条消息再读一次，此后没有读取
  await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_READ_MS); });
  expect(reads).toBe(5);
  await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_READ_MS * 2); });
  expect(reads).toBe(5);
});
