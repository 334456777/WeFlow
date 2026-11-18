import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { Profile } from '../api/setup';
import { AiSetting } from './AiSettings';

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

describe('AI settings', () => {
  it('saves only the AI settings changed, sends the key without ever reading it, and tests the saved ones', async () => {
    const values: Record<string, unknown> = { ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'm1', ai_model_api_max_tokens: null, ai_insight_enabled: true };
    const calls = serve((_method, url) => {
      const key = new URL(url, 'http://x').searchParams.get('key');
      if (url.startsWith('/api/config/get')) return ok(values[key!] ?? null);
      if (url === '/insight/test') return json({ success: false, error: { code: 'runtime_error', message: 'HTTP 401 from the model API' } }, 500);
    });
    const onSaved = vi.fn();
    render(<AiSetting profile={PROFILE} engine={false} onSaved={onSaved} onServe={() => {}} />);
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
});
