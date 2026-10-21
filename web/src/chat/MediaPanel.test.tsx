import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from '#test';
import { emitLive, LIVE_COMMAND_MS } from '../api/live';
import { MediaPanel } from './MediaPanel';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });
// a page goes on from the cursor of the last item of the one before (#178)
const json = (items: { _cursor: string }[], hasMore: boolean) =>
  new Response(JSON.stringify({ success: true, data: { items, hasMore, nextCursor: hasMore ? items.at(-1)!._cursor : null } }));
const item = (id: number) => ({ _cursor: 'c' + id, localId: id, createTime: 1760000000 + id, localType: 25769803825, resourceType: 'file', fileName: '文件' + id });

it('pages media from the cursor of the page before and cancels the next page when its card closes', async () => {
  let signal: AbortSignal | undefined;
  const fetch = vi.fn().mockResolvedValueOnce(json(Array.from({ length: 60 }, (_, i) => item(i)), true))
    .mockResolvedValueOnce(json(Array.from({ length: 60 }, (_, i) => item(60 + i)), true))
    .mockImplementationOnce((_url: string, init: RequestInit) => {
      signal = init.signal as AbortSignal;
      return new Promise((_resolve, reject) => signal?.addEventListener('abort', () => reject(signal?.reason), { once: true }));
    });
  vi.stubGlobal('fetch', fetch);
  const props = { sessionId: 'person', onOpen: vi.fn() };
  const view = render(<MediaPanel {...props} />);
  await act(async () => {});
  expect(document.querySelectorAll('.media-item')).toHaveLength(60);
  fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
  await act(async () => {});
  expect(fetch.mock.calls[1][0]).toContain('before=c59');
  expect(document.querySelectorAll('.media-item')).toHaveLength(120);
  fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
  await act(async () => {});
  view.rerender(<MediaPanel {...props} active={false} />);
  await act(async () => {});
  expect(signal?.aborted).toBe(true);
});

it('reads a burst of live messages once and keeps the older pages shown (#201)', async () => {
  vi.useFakeTimers();
  const page = (from: number) => json(Array.from({ length: 60 }, (_, i) => item(from - i)), true);
  // newest first: 200…141, then 140…81; then two new files, which push 142 and 141 onto the second page
  const fetch = vi.fn().mockResolvedValueOnce(page(200)).mockResolvedValueOnce(page(140))
    .mockResolvedValueOnce(page(202)).mockResolvedValueOnce(page(80));
  vi.stubGlobal('fetch', fetch);
  render(<MediaPanel sessionId="person" onOpen={vi.fn()} />);
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
  await act(async () => {});
  expect(document.querySelectorAll('.media-item')).toHaveLength(120);

  act(() => { for (let i = 0; i < 3; i++) emitLive({ type: 'message.new', sessionId: 'person' }); });
  emitLive({ type: 'message.new', sessionId: 'someone else' });
  await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_COMMAND_MS - 1); });
  expect(fetch).toHaveBeenCalledTimes(2);
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(fetch.mock.calls[2][0]).not.toContain('before=');
  const names = () => [...document.querySelectorAll('.media-item .clamp-1')].map(e => e.textContent);
  expect(names()).toHaveLength(122);
  expect(names().slice(0, 3)).toEqual(['文件202', '文件201', '文件200']);
  expect(new Set(names()).size).toBe(122);

  // the next page goes on from where the list ends, which the new files did not move
  fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
  await act(async () => {});
  expect(fetch.mock.calls[3][0]).toContain('before=c81');
  expect(names().at(-1)).toBe('文件21');
});
