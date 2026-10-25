// The requests the page makes in the flows people go through most (#403): the baseline that #402 changes on
// purpose. Each step lists what the page asked the server for, in order (`fields` left out, as it only picks
// columns); a change of these lists is a change of the page's requests, to be explained in its PR
// (docs/refactor-baseline.md).
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { App } from './App';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
  streams.length = 0;
});

const json = (body: unknown) => new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } });

/** Two accounts of one server, told apart by the `data` of `/health/events` (see api/scope.ts). */
const ACCOUNTS: Record<string, { username: string; displayName: string }[]> = {
  a: [{ username: 'wxid_lin', displayName: '林沐' }, { username: 'wxid_zhou', displayName: '周舟' }],
  b: [{ username: 'wxid_qin', displayName: '秦川' }],
};

let account = 'a';
let log: string[] = [];
const streams: FakeEventSource[] = [];

class FakeEventSource {
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((event: MessageEvent) => void)[]>();
  constructor(readonly url: string) {
    streams.push(this);
    log.push('stream ' + url.split('?')[0]);
    queueMicrotask(() => {
      if (!this.closed && !url.startsWith('/jobs/')) this.emit('hello', { status: 'ok', apiVersion: '1', version: '5.0.1', data: account });
    });
  }
  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() { this.closed = true; }
  emit(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(new MessageEvent(type, { data: JSON.stringify(data) }));
  }
}

function message(session: string, localId: number) {
  return { _cursor: `${session}.${localId}`, localId, localType: 1, createTime: 1760000000 + localId, isSend: 0, senderUsername: session, parsedContent: `${account} 的消息 ${localId}` };
}

function answer(url: string): Response {
  const [path, query = ''] = url.split('?');
  const params = new URLSearchParams(query);
  switch (path) {
    case '/jobs':
      return json({ success: true, data: [] });
    case '/config/profiles':
      return json({ success: true, data: [{ name: 'default', current: true, complete: true }] });
    case '/api/chat/sessions':
      return json({ success: true, data: ACCOUNTS[account].map((s, i) => ({ ...s, summary: '', unreadCount: 0, sortTimestamp: 1760000000 - i, lastTimestamp: 1760000000 - i, lastMsgType: 1 })) });
    case '/chat/statuses':
      return json({ success: true, data: {} });
    case '/api/chat/messages':
      return json({ success: true, data: [message(params.get('session-id') ?? '', 2), message(params.get('session-id') ?? '', 1)] });
    case '/api/chat/search':
      return json({ success: true, data: [] });
  }
  throw new TypeError('unexpected ' + url);
}

/** A request as the lists show it: the method and the address without `fields`. */
function shown(url: string, init?: RequestInit): string {
  const [path, query = ''] = url.split('?');
  const params = new URLSearchParams(query);
  params.delete('fields');
  const rest = decodeURIComponent(params.toString());
  return (init?.method ?? 'GET') + ' ' + path + (rest ? '?' + rest : '');
}

function serve(late?: (url: string) => boolean) {
  const held: (() => void)[] = [];
  vi.stubGlobal('EventSource', FakeEventSource);
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    log.push(shown(url, init));
    const reply = answer(url);
    if (late?.(url)) await new Promise<void>(resolve => held.push(resolve));
    return reply;
  }));
  return { release: () => held.splice(0).forEach(resolve => resolve()) };
}

/** The requests of `step`, from when it starts until the page is quiet. */
async function requestsOf(step: () => void): Promise<string[]> {
  log = [];
  await act(async () => { step(); });
  await act(async () => {});
  await act(async () => {});
  return log;
}

/** The server says its data changed: the configuration names account `next` now. */
function switchAccount(next: string) {
  account = next;
  for (const stream of streams.filter(s => !s.closed && !s.url.startsWith('/jobs/'))) stream.emit('heartbeat', { data: next });
}

const conversations = () => Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent);
const messages = () => Array.from(document.querySelectorAll('.message-pane .message-row'), row => row.textContent ?? '');
const PAGE = 'limit=50&parsed=true';

describe('requests of the chat page', () => {
  it('at the start, for another conversation, back, a search and another account', async () => {
    account = 'a';
    serve();

    expect(await requestsOf(() => { render(<App />); })).toEqual([
      'stream /api/push/messages',
      // the token check, and whether the account is set up
      'GET /jobs',
      'GET /config/profiles',
      'GET /api/chat/sessions',
      'GET /chat/statuses?usernames=wxid_lin&usernames=wxid_zhou',
      `GET /api/chat/messages?session-id=wxid_lin&${PAGE}`,
    ]);
    expect(await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择周舟' })); })).toEqual([
      `GET /api/chat/messages?session-id=wxid_zhou&${PAGE}`,
    ]);
    // a conversation opened before is kept
    expect(await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择林沐' })); })).toEqual([]);
    expect(await requestsOf(() => {
      fireEvent.change(screen.getByRole('searchbox', { name: '搜索此会话的消息' }), { target: { value: '消息' } });
      fireEvent.click(screen.getByRole('button', { name: '搜索' }));
    })).toEqual([`GET /api/chat/search?keyword=消息&session-id=wxid_lin&limit=20&offset=0&parsed=true`]);

    expect(await requestsOf(() => switchAccount('b'))).toEqual([
      // the conversation open before, asked of the new account before its list is read (for #402)
      `GET /api/chat/messages?session-id=wxid_lin&${PAGE}`,
      'GET /jobs',
      'GET /config/profiles',
      'GET /api/chat/sessions',
      'GET /chat/statuses?usernames=wxid_qin',
      `GET /api/chat/messages?session-id=wxid_qin&${PAGE}`,
    ]);
    expect(conversations()).toEqual(['秦川']);
    expect(messages().every(text => text.startsWith('b 的消息'))).toBe(true);
  });

  it('keeps an answer of the old account that comes after the switch out of the page', async () => {
    account = 'a';
    const server = serve(url => url.includes('session-id=wxid_zhou'));
    await requestsOf(() => { render(<App />); });
    await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择周舟' })); });
    await requestsOf(() => switchAccount('b'));
    expect(conversations()).toEqual(['秦川']);
    // 周舟's page of account a comes back now
    await requestsOf(() => server.release());
    expect(conversations()).toEqual(['秦川']);
    expect(messages().length).toBeGreaterThan(0);
    expect(messages().every(text => text.startsWith('b 的消息'))).toBe(true);
  });
});
