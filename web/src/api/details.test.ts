import { afterEach, expect, it, vi } from '#test';
import { getDetailExtra } from './details';

const noExtra = { firstMessageTime: null, latestMessageTime: null, messageTables: [] };

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });
const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }));

it('serializes command reads and skips a closed card while it waits', async () => {
  let finish!: (response: Response) => void;
  const fetch = vi.fn().mockImplementationOnce(() => new Promise<Response>(resolve => { finish = resolve; }))
    .mockResolvedValue(json(null));
  vi.stubGlobal('fetch', fetch);
  const first = getDetailExtra('group');
  const controller = new AbortController();
  const closed = getDetailExtra('person', controller.signal).catch(error => error);
  const last = getDetailExtra('room');
  await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  controller.abort();
  finish(json({ firstMessageTime: 1, latestMessageTime: 2, messageTables: [] }));
  await first;
  expect((await closed).name).toBe('AbortError');
  expect(await last).toEqual(noExtra);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch.mock.calls[1][0]).toContain('session-id=room');
});

it('waits briefly when another command holds the slot, then retries', async () => {
  vi.useFakeTimers();
  const fetch = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({ success: false, error: { code: 'busy' } }), { status: 429 }))
    .mockResolvedValueOnce(json(null));
  vi.stubGlobal('fetch', fetch);
  const read = getDetailExtra('person');
  await vi.advanceTimersByTimeAsync(1000);
  expect(await read).toEqual(noExtra);
  expect(fetch).toHaveBeenCalledTimes(2);
});
