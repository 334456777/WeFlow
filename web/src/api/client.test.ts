import { afterEach, describe, expect, it, vi } from 'vitest';
import { adoptToken, ApiError, apiUrl, call, checkToken, onAuth, setToken, token } from './client';

function answer(status: number, body: unknown): Response {
  return new Response(typeof body === 'string' ? body : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

afterEach(() => {
  vi.unstubAllGlobals();
  setToken(null);
  window.history.replaceState(null, '', '/ui/');
});

describe('apiUrl', () => {
  it('repeats list values and leaves out empty ones', () => {
    expect(apiUrl('api/chat/messages', { 'session-id': 'a b', limit: 20, before: undefined, keyword: '', fast: true }))
      .toBe('/api/chat/messages?session-id=a+b&limit=20&fast=true');
    expect(apiUrl('/chat/contacts', { type: ['friend', 'group'] })).toBe('/chat/contacts?type=friend&type=group');
    expect(apiUrl('health')).toBe('/health');
  });
});

describe('the token', () => {
  it('ignores a late refusal of the token replaced while a request was in flight', async () => {
    const replies: ((response: Response) => void)[] = [];
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(resolve => replies.push(resolve))));
    setToken('old');
    const events: string[] = [];
    const off = onAuth(event => events.push(event));
    try {
      const request = call('GET', 'api/chat/sessions');
      const probe = checkToken();
      setToken('new');
      for (const reply of replies) reply(answer(401, { success: false, error: { code: 'unauthorized' } }));
      await expect(request).rejects.toMatchObject({ status: 401 });
      await expect(probe).resolves.toBe('offline');
      expect(events).toEqual(['token']);
    } finally { off(); }
  });

  it('is taken from the address and removed from it', () => {
    window.history.replaceState(null, '', '/ui/chat?token=secret&x=1#top');
    adoptToken();
    expect(token()).toBe('secret');
    expect(sessionStorage.getItem('weflow.token')).toBe('secret');
    expect(window.location.pathname + window.location.search + window.location.hash).toBe('/ui/chat?x=1#top');
  });

  it('is sent as a bearer token', async () => {
    setToken('t1');
    const fetch = vi.fn().mockResolvedValue(answer(200, { success: true, data: [] }));
    vi.stubGlobal('fetch', fetch);
    await call('GET', 'api/chat/sessions');
    expect(fetch.mock.calls[0][1].headers.Authorization).toBe('Bearer t1');
  });
});

describe('call', () => {
  it('returns the data of a successful answer', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(answer(200, { success: true, data: { n: 1 } })));
    await expect(call('GET', 'api/chat/counts')).resolves.toEqual({ n: 1 });
  });

  it('sends the parameters of other methods as a JSON body', async () => {
    const fetch = vi.fn().mockResolvedValue(answer(200, { success: true, data: null }));
    vi.stubGlobal('fetch', fetch);
    await call('PUT', 'config/set', { key: 'no_progress', value: 'true' });
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe('/config/set');
    expect(init.method).toBe('PUT');
    expect(JSON.parse(init.body)).toEqual({ key: 'no_progress', value: 'true' });
  });

  it('throws the error code of a failed answer', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(answer(401, { success: false, error: { code: 'unauthorized', message: 'invalid or missing access token' } })));
    await expect(call('GET', 'chat/sessions')).rejects.toMatchObject({ status: 401, code: 'unauthorized' });
  });

  it('tells a missing server and an answer that is not JSON apart', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
    await expect(call('GET', 'chat/sessions')).rejects.toMatchObject({ status: 0, code: 'offline' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(answer(502, 'Bad Gateway')));
    const error = await call('GET', 'chat/sessions').catch(e => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 502, code: 'bad_response' });
  });
});
