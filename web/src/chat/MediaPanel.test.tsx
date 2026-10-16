import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MediaPanel } from './MediaPanel';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const json = (items: unknown[], hasMore: boolean) => new Response(JSON.stringify({ success: true, data: { items, hasMore } }));
const item = (id: number) => ({ localId: id, createTime: 1760000000 + id, localType: 25769803825, resourceType: 'file', fileName: '文件' + id });

it('pages media with the shown offset and cancels the next page when its card closes', async () => {
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
  expect(fetch.mock.calls[1][0]).toContain('offset=60');
  expect(document.querySelectorAll('.media-item')).toHaveLength(120);
  fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
  await act(async () => {});
  view.rerender(<MediaPanel {...props} active={false} />);
  await act(async () => {});
  expect(signal?.aborted).toBe(true);
});
