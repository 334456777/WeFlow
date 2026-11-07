import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { ServeOptions } from './ServeOptions';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const ok = (data: unknown) => json({ success: true, data });

function serve(saved: Record<string, unknown>, refuse?: string) {
  const calls: { method: string; url: string; body?: unknown }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ method, url, body });
    if (url.startsWith('/api/config/get')) return ok(saved[new URL(url, 'http://x').searchParams.get('key')!] ?? null);
    if (refuse && body?.key === refuse) return json({ success: false, error: { code: 'invalid_arguments', message: `invalid value for ${refuse}` } }, 400);
    return ok({});
  }));
  return calls;
}

describe('the options weflow serve starts with (#406)', () => {
  it('saves only what changed, a default as nothing, and warns of an address beyond this machine', async () => {
    const calls = serve({ serve_port: 8080, serve_open_browser: false });
    render(<ServeOptions />);
    await act(async () => {});
    expect((screen.getByPlaceholderText('18108') as HTMLInputElement).value).toBe('8080');
    const browser = screen.getByRole('checkbox', { name: '启动时在浏览器中打开网页' }) as HTMLInputElement;
    expect(browser.checked).toBe(false);
    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText('127.0.0.1'), { target: { value: '0.0.0.0' } });
    expect(screen.getByText('监听本机以外的地址时，持有令牌的人可以在这台机器上运行任何 weflow 命令。')).toBeTruthy();
    fireEvent.change(screen.getByPlaceholderText('18108'), { target: { value: '' } });
    fireEvent.click(browser);
    fireEvent.click(screen.getByRole('checkbox', { name: '运行灵感引擎' }));
    fireEvent.click(save);
    await act(async () => {});
    expect(calls.filter(c => c.method !== 'GET')).toEqual([
      { method: 'PUT', url: '/config/set', body: { key: 'serve_host', value: '0.0.0.0' } },
      { method: 'DELETE', url: '/config/unset?key=serve_port', body: undefined },
      { method: 'DELETE', url: '/config/unset?key=serve_open_browser', body: undefined },
      { method: 'PUT', url: '/config/set', body: { key: 'serve_insight', value: 'true' } },
    ]);
    expect(screen.getByText('已保存，重启 weflow serve 后生效。')).toBeTruthy();
    expect(save.disabled).toBe(true);
  });

  it('checks the numbers, and shows what the server refused', async () => {
    serve({}, 'serve_host');
    render(<ServeOptions />);
    await act(async () => {});
    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement;
    fireEvent.change(screen.getByPlaceholderText('1'), { target: { value: '0' } });
    expect(screen.getByText('要填 1 到 65535 的整数。')).toBeTruthy();
    expect(save.disabled).toBe(true);
    fireEvent.change(screen.getByPlaceholderText('1'), { target: { value: '2' } });
    fireEvent.change(screen.getByPlaceholderText('127.0.0.1'), { target: { value: 'localhost' } });
    fireEvent.click(save);
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('invalid value for serve_host');
  });
});
