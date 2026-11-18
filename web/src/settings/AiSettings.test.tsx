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

/** The settings `values`, as `config get` reads them. */
const settings = (values: Record<string, unknown>) => (url: string) =>
  url.startsWith('/api/config/get') ? ok(values[new URL(url, 'http://x').searchParams.get('key')!] ?? null) : undefined;

const PROFILE: Profile = { name: 'default', current: true, wxid: 'wxid_a', dbPath: 'D:/x', decryptKey: true, imageKeys: true, aiKey: true, complete: true };

describe('AI settings', () => {
  it('saves only the AI settings changed, sends the key without ever reading it, and tests the saved ones', async () => {
    const read = settings({ ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'm1', ai_model_api_max_tokens: null, ai_insight_enabled: true });
    const calls = serve((_method, url) => {
      if (url === '/insight/test') return json({ success: false, error: { code: 'runtime_error', message: 'HTTP 401 from the model API' } }, 500);
      return read(url);
    });
    const onSaved = vi.fn();
    render(<AiSetting profile={PROFILE} engine={false} onSaved={onSaved} onServe={() => {}} />);
    await act(async () => {});
    expect(calls.every(c => !c.url.includes('ai_model_api_key'))).toBe(true);
    expect((screen.getByLabelText('API Key') as HTMLInputElement).placeholder).toBe('已设置；留空则不修改');
    expect((screen.getByRole('checkbox', { name: '开启 AI 见解' }) as HTMLInputElement).checked).toBe(true);
    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText('模型'), { target: { value: 'm2' } });
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

    // the server's own words, which say what failed
    fireEvent.click(screen.getByRole('button', { name: '测试连接' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('HTTP 401 from the model API');
  });

  it('shows where a question goes in each format, and saves the format chosen', async () => {
    const calls = serve((_method, url) => settings({ ai_model_api_base_url: '', ai_model_api_model: 'm1' })(url));
    render(<AiSetting profile={PROFILE} engine={false} onSaved={() => {}} onServe={() => {}} />);
    await act(async () => {});
    const openai = screen.getByRole('button', { name: /OpenAI 兼容/ });
    const anthropic = screen.getByRole('button', { name: /Anthropic 兼容/ });
    // not set: OpenAI's, at its own address
    expect(openai.getAttribute('aria-pressed')).toBe('true');
    expect(document.querySelector('.ai-endpoint')?.textContent).toContain('POST https://api.openai.com/v1/chat/completions');

    fireEvent.click(anthropic);
    expect(anthropic.getAttribute('aria-pressed')).toBe('true');
    expect(document.querySelector('.ai-endpoint')?.textContent).toContain('POST https://api.anthropic.com/v1/messages');
    expect((screen.getByLabelText('模型') as HTMLInputElement).placeholder).toBe('例如 claude-opus-5-5');
    fireEvent.change(screen.getByPlaceholderText('https://api.anthropic.com'), { target: { value: 'https://gw.example.com/v1/' } });
    expect(document.querySelector('.ai-endpoint')?.textContent).toContain('POST https://gw.example.com/v1/messages');

    // back to the format the setting means: nothing to save
    fireEvent.click(openai);
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), { target: { value: '' } });
    expect((screen.getByRole('button', { name: '保存' }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(anthropic);
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await act(async () => {});
    expect(calls.filter(c => c.method === 'PUT').map(c => c.body)).toEqual([{ key: 'ai_model_api_format', value: 'anthropic' }]);
  });

  it('shows what the test found, and offers the models the service lists', async () => {
    serve((_method, url) => {
      if (url === '/insight/test') return ok({ success: true, message: '连接成功', probe: {
        format: 'openai', endpoint: 'https://api.example.com/v1/chat/completions', model: 'm1-2026', reply: '连接成功', durationMs: 1234, retries: 1,
      } });
      if (url === '/insight/models') return ok({ format: 'openai', endpoint: 'https://api.example.com/v1/models', models: [{ id: 'm1' }, { id: 'm2', displayName: 'Model Two' }] });
      return settings({ ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'm1' })(url);
    });
    render(<AiSetting profile={PROFILE} engine={false} onSaved={() => {}} onServe={() => {}} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '测试连接' }));
    await act(async () => {});
    const found = screen.getByRole('status');
    expect(found.textContent).toContain('已连接');
    expect(found.textContent).toContain('m1-2026');
    expect(found.textContent).toContain('1.2 秒');
    expect(found.textContent).toContain('重试 1 次后成功');
    expect(found.textContent).toContain('https://api.example.com/v1/chat/completions');

    fireEvent.click(screen.getByRole('button', { name: '获取模型列表' }));
    await act(async () => {});
    const list = screen.getByRole('group', { name: '服务提供的模型（2 个）' });
    expect(list.querySelector('[aria-pressed="true"]')?.textContent).toBe('m1');
    fireEvent.click(screen.getByRole('button', { name: /m2/ }));
    expect((screen.getByLabelText('模型') as HTMLInputElement).value).toBe('m2');
    expect(list.querySelector('[aria-pressed="true"]')?.textContent).toContain('Model Two');
  });
});
