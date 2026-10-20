import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Profile } from '../api/setup';
import { AiSetting, CacheSetting, LanguageSetting } from './MoreSettings';

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

const PROFILE: Profile = { name: 'default', current: true, wxid: 'wxid_a', dbPath: 'D:/x', decryptKey: true, imageKeys: true, aiKey: true, complete: true };

describe('settings', () => {
  it('saves the output language, and follows the system again by unsetting it', async () => {
    const calls = serve((_method, url) => (url === '/config/get?key=lang' ? ok('zh') : undefined));
    render(<LanguageSetting />);
    await act(async () => {});
    expect(screen.getByRole('button', { name: '中文' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'English' }));
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '跟随系统' }));
    await act(async () => {});
    expect(calls.slice(1)).toEqual([
      { method: 'PUT', url: '/config/set', body: { key: 'lang', value: 'en' } },
      { method: 'DELETE', url: '/config/unset?key=lang', body: undefined },
    ]);
  });

  it('saves only the AI settings changed, sends the key without ever reading it, and tests the saved ones', async () => {
    const values: Record<string, unknown> = { ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'm1', ai_model_api_max_tokens: null, ai_insight_enabled: true };
    const calls = serve((_method, url) => {
      const key = new URL(url, 'http://x').searchParams.get('key');
      if (url.startsWith('/config/get')) return ok(values[key!] ?? null);
      if (url === '/insight/test') return json({ success: false, error: { code: 'runtime_error', message: 'HTTP 401 from the model API' } }, 500);
    });
    const onSaved = vi.fn();
    render(<AiSetting profile={PROFILE} onSaved={onSaved} />);
    await act(async () => {});
    expect(calls.every(c => !c.url.includes('ai_model_api_key'))).toBe(true);
    expect((screen.getByLabelText('API Key') as HTMLInputElement).placeholder).toBe('已设置；留空则不修改');
    expect((screen.getByRole('checkbox', { name: '开启 AI 见解' }) as HTMLInputElement).checked).toBe(true);
    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText('模型名称'), { target: { value: 'm2' } });
    fireEvent.change(screen.getByLabelText('API Key'), { target: { value: 'sk-new' } });
    // a change waits for its save before a test
    expect((screen.getByRole('button', { name: '测试连接' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(save);
    await act(async () => {});
    expect(calls.filter(c => c.method === 'PUT').map(c => c.body)).toEqual([
      { key: 'ai_model_api_model', value: 'm2' }, { key: 'ai_model_api_key', value: 'sk-new' },
    ]);
    expect(onSaved).toHaveBeenCalled();
    expect((screen.getByLabelText('API Key') as HTMLInputElement).value).toBe('');

    fireEvent.click(screen.getByRole('button', { name: '测试连接' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('连接失败：HTTP 401 from the model API');
  });

  it('lists the cache by part, and clears one after showing what goes', async () => {
    let images = 3 * 1024 * 1024;
    const calls = serve((_method, url) => {
      if (url === '/cache/list') return ok([{ part: 'images', paths: ['C:/c/Images'], bytes: images, files: 12 }, { part: 'voices', paths: ['C:/c/Voices'], bytes: 0, files: 0 }]);
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
    // under the row of the button pressed, not after the whole list (#346)
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
