import { afterEach, expect, it, vi } from 'vitest';
import { getDetailExtra, getResources } from './details';

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });
const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }));

it('serializes command reads and skips a closed card while it waits', async () => {
  let finish!: (response: Response) => void;
  const fetch = vi.fn().mockImplementationOnce(() => new Promise<Response>(resolve => { finish = resolve; }))
    .mockResolvedValue(json({ items: [], hasMore: false }));
  vi.stubGlobal('fetch', fetch);
  const first = getDetailExtra('group');
  const controller = new AbortController();
  const closed = getResources('group', ['image'], 0, controller.signal).catch(error => error);
  const last = getResources('group', ['video'], 0);
  await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  controller.abort();
  finish(json({ firstMessageTime: 1, latestMessageTime: 2, messageTables: [] }));
  await first;
  expect((await closed).name).toBe('AbortError');
  expect(await last).toEqual({ items: [], hasMore: false });
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch.mock.calls[1][0]).toContain('type=video');
});

it('waits briefly when another command holds the slot, then retries', async () => {
  vi.useFakeTimers();
  const fetch = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({ success: false, error: { code: 'busy' } }), { status: 429 }))
    .mockResolvedValueOnce(json({ items: [], hasMore: false }));
  vi.stubGlobal('fetch', fetch);
  const read = getResources('person', ['file'], 0);
  await vi.advanceTimersByTimeAsync(1000);
  expect(await read).toEqual({ items: [], hasMore: false });
  expect(fetch).toHaveBeenCalledTimes(2);
});
