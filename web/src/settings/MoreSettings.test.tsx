import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { CacheSetting } from './MoreSettings';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const ok = (data: unknown) => json({ success: true, data });

function serve(answer: (method: string, url: string) => Response | undefined) {
  const calls: { method: string; url: string; body?: unknown }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    return answer(method, url) ?? ok({});
  }));
  return calls;
}

describe('settings', () => {
  it('lists the cache by part, and clears one after showing what goes', async () => {
    let images = 3 * 1024 * 1024;
    const calls = serve((_method, url) => {
      if (url === '/api/cache/list') return ok([{ part: 'images', paths: ['C:/c/Images'], bytes: images, files: 12 }, { part: 'voices', paths: ['C:/c/Voices'], bytes: 0, files: 0 }]);
      if (url === '/cache/clear?images=true&dry-run=true') return ok({ dryRun: true, paths: [{ path: 'C:/c/Images', bytes: images }], bytes: images, profileKeys: [] });
      if (url === '/cache/clear?images=true&yes=true') { images = 0; return ok({ removedPaths: ['C:/c/Images'], freedBytes: 3 }); }
    });
    render(<CacheSetting />);
    await act(async () => {});
    expect(document.querySelector('.cache-list')?.textContent).toContain('3.0 MB · 12 个文件');
    expect((screen.getByRole('button', { name: '清理语音' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '清理图片' }));
    await act(async () => {});
    const plan = screen.getByRole('group', { name: '将要清理的内容' });
    expect(plan.textContent).toContain('C:/c/Images');
    // 显示在按下的按钮那一行下面，而不是整个列表后面（#346）
    expect(plan.closest('li')).toBe(screen.getByRole('button', { name: '清理图片' }).closest('li'));
    expect(calls.some(c => c.url.includes('yes=true'))).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '确认清理' }));
    await act(async () => {});
    expect(calls.filter(c => c.url.startsWith('/cache/clear')).map(c => c.method + ' ' + c.url)).toEqual([
      'DELETE /cache/clear?images=true&dry-run=true', 'DELETE /cache/clear?images=true&yes=true',
    ]);
    expect(document.querySelector('.cache-list')?.textContent).toContain('图片');
    expect(screen.getByText(/^已清理图片/).closest('li')).toBe(screen.getByRole('button', { name: '清理图片' }).closest('li'));
    expect((screen.getByRole('button', { name: '清理图片' }) as HTMLButtonElement).disabled).toBe(true);
  });
});
