import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { WEB, type Health } from '../api/backend';
import { WebVersions } from './WebVersions';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const ok = (data: unknown) => json({ success: true, data });

const kept = (version: string, extra: object = {}) => ({ version, apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion, path: 'x', inUse: false, picked: false, suits: true, ...extra });
const HEALTH: Health = { status: 'ok', apiVersion: WEB.apiVersion, version: '9.9.9' };
const published = (version: string, extra: object = {}) => ({ version, size: 2_000_000, apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion, suits: true, why: null, downloaded: false, ...extra });

function serve(answer: (method: string, url: string) => Response | undefined) {
  const calls: { method: string; url: string; body?: unknown }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    return answer(method, url) ?? ok({});
  }));
  return calls;
}

describe('the WebUI versions (#406)', () => {
  it('lists the versions kept, then the published ones, and switches to one, reloading when the server shows it', async () => {
    let shown = WEB.version;
    const calls = serve((method, url) => {
      if (url === '/web/list') return ok([kept(WEB.version, { inUse: true })]);
      if (url === '/web/list?remote=true') return ok([published('9.0.0-rc.1'), published('8.0.0'), published('7.0.0', { suits: false, why: 'it needs API 1', apiVersion: '1' }), published(WEB.version, { downloaded: true })]);
      if (method === 'PUT' && url === '/web/use') { shown = '8.0.0'; return ok({ version: '8.0.0', path: 'x' }); }
      if (url === '/ui/weflow-web.json') return json({ version: shown });
    });
    const onReload = vi.fn();
    render(<WebVersions health={HEALTH} onReload={onReload} />);
    await act(async () => {});
    expect(screen.getAllByRole('listitem').map(li => li.textContent)).toEqual([WEB.version + '正在使用 · 已下载']);

    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    await act(async () => {});
    expect(screen.getByText('有新的适用版本 8.0.0。')).toBeTruthy();
    const items = screen.getAllByRole('listitem').map(li => li.textContent);
    // a release before a pre-release, as `web update` picks
    expect(items[1]).toBe('8.0.0最新 · 1.9 MB换上');
    expect(items[2]).toBe(`7.0.0需要 API 1，这个 weflow 提供 API ${WEB.apiVersion}`);
    expect(screen.queryByRole('button', { name: '换成 7.0.0' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '换成 8.0.0' }));
    await act(async () => {});
    expect(calls.find(c => c.method === 'PUT')).toEqual({ method: 'PUT', url: '/web/use', body: { version: '8.0.0' } });
    expect(onReload).toHaveBeenCalledTimes(1);
  });

  it('says when the server shows another page this run, and what prune removed and left', async () => {
    serve((method, url) => {
      // 0.0.5: a version the page itself (WEB.version) never has, so the rows stay three
      if (url === '/web/list') return ok([kept(WEB.version), kept('1.0.0'), kept('0.0.5', { suits: false, minBackendVersion: '99.0.0' })]);
      if (method === 'PUT') return ok({ version: '1.0.0', path: 'x' });
      if (url === '/ui/weflow-web.json') return json({ version: WEB.version });
      if (method === 'DELETE' && url === '/web/prune') return ok({ removed: ['1.0.0'], kept: [WEB.version], shown: ['0.9.0'] });
    });
    const onReload = vi.fn();
    render(<WebVersions health={HEALTH} onReload={onReload} />);
    await act(async () => {});
    expect(screen.getAllByRole('listitem')[2].textContent).toBe('0.0.5需要 weflow 99.0.0 或更新的版本，这个是 9.9.9');
    fireEvent.click(screen.getByRole('button', { name: '换成 1.0.0' }));
    await act(async () => {});
    expect(onReload).not.toHaveBeenCalled();
    expect(screen.getByText('已选定 1.0.0。这次 weflow serve 指定了要显示的网页，下次启动时换上。')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '清理旧版本' }));
    await act(async () => {});
    expect(screen.getByText('删除了 1 个版本：1.0.0。正在显示的 0.9.0 等 weflow serve 停止后再删除。')).toBeTruthy();
  });

  it('shows the error of the registry', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => url === '/web/list?remote=true'
      ? json({ success: false, error: { code: 'runtime_error', message: 'cannot reach the registry' } }, 500)
      : ok([])));
    render(<WebVersions health={HEALTH} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('cannot reach the registry');
  });
});
