// The requests the page makes in the flows people go through most (#403). Each step lists what the page asked the
// server for, in order (`fields` left out, as it only picks columns); a change of these lists is a change of the page's requests, to be explained in its PR (see #403).
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { App } from './App';
import { LIVE_READ_MS } from './api/live';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
  streams.length = 0;
  long = false;
  joined = false;
});

const json = (body: unknown) => new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } });

/** Two accounts of one server, told apart by the `data` of `/health/events` (see api/scope.ts). */
const ACCOUNTS: Record<string, { username: string; displayName: string }[]> = {
  a: [{ username: 'wxid_lin', displayName: '林沐' }, { username: 'wxid_zhou', displayName: '周舟' }],
  b: [{ username: 'wxid_qin', displayName: '秦川' }],
  g: [{ username: 'team@chatroom', displayName: '设计群' }],
};

/** Whether 新同学 joined the group of account `g` (and 林沐 renamed themselves in it), after its first page. */
let joined = false;

let account = 'a';
let log: string[] = [];
/** Whether the first page of a conversation is a full one, so that older pages are left to read. */
let long = false;
const streams: FakeEventSource[] = [];

class FakeEventSource {
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((event: MessageEvent) => void)[]>();
  constructor(readonly url: string) {
    streams.push(this);
    log.push('stream ' + url.split('?')[0]);
    queueMicrotask(() => {
      if (!this.closed && !url.startsWith('/jobs/')) this.emit('hello', { status: 'ok', apiVersion: __WEFLOW_WEB__.apiVersion, version: __WEFLOW_WEB__.minBackendVersion, data: account });
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
    case '/api/config/profiles':
      return json({ success: true, data: [{ name: 'default', current: true, complete: true }] });
    case '/api/chat/sessions':
      return json({ success: true, data: ACCOUNTS[account].map((s, i) => ({ ...s, summary: '', unreadCount: 0, sortTimestamp: 1760000000 - i, lastTimestamp: 1760000000 - i, lastMsgType: 1 })) });
    case '/api/chat/statuses':
      return json({ success: true, data: {} });
    case '/api/chat/messages': {
      const session = params.get('session-id') ?? '';
      if (session === 'team@chatroom') {
        // 林沐's two messages; once 新同学 joined, theirs after them
        const said = (localId: number, sender: string) => ({ ...message(session, localId), senderUsername: sender });
        if (params.has('after')) return json({ success: true, data: joined && params.get('after') !== `${session}.3` ? [said(3, 'wxid_new')] : [] });
        return json({ success: true, data: [said(2, 'wxid_lin'), said(1, 'wxid_lin')] });
      }
      const count = long && !params.has('before') ? 50 : 2;
      const from = params.has('before') ? 0 : 50;
      return json({ success: true, data: Array.from({ length: count }, (_, i) => message(session, from + count - i)) });
    }
    case '/api/group/members':
      return json({ success: true, data: { members: [
        { username: 'wxid_lin', displayName: '林沐', groupNickname: joined ? '林林' : '小林' },
        ...joined ? [{ username: 'wxid_new', displayName: '新同学', groupNickname: '' }] : [],
      ] } });
    case '/api/chat/contacts':
      return json({ success: true, data: ACCOUNTS[account].map(s => ({ ...s, type: 'friend' })) });
    case '/api/insight/footprint':
      return json({ success: true, data: { summary: { private_inbound_people: 0, private_replied_people: 0, private_outbound_people: 0, private_reply_rate: 0, mention_count: 0, mention_group_count: 0 }, private_sessions: [], private_segments: [], mentions: [], mention_groups: [], diagnostics: { truncated: false } } });
    case '/api/sns/timeline':
      return json({ success: true, data: [] });
    case '/api/sns/stats':
    case '/api/sns/post-counts':
      return json({ success: true, data: {} });
    case '/api/chat/search': {
      const keyword = params.get('keyword') ?? '';
      return json({ success: true, data: [{ ...message(params.get('session') ?? '', 1), _session_id: params.get('session'), _cursor: 'hit.' + keyword, parsedContent: '说到' + keyword }] });
    }
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

const hits = () => Array.from(document.querySelectorAll('.search-result'), hit => hit.textContent ?? '');
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
      'GET /api/config/profiles',
      'GET /api/chat/sessions',
      'GET /api/chat/statuses?sessions=wxid_lin&sessions=wxid_zhou',
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
    })).toEqual([`GET /api/chat/search?keyword=消息&session=wxid_lin&limit=20&offset=0&parsed=true`]);

    // nothing of the account before is asked of the new one: no conversation is open until its list is read (#402)
    expect(await requestsOf(() => switchAccount('b'))).toEqual([
      'GET /jobs',
      'GET /api/config/profiles',
      'GET /api/chat/sessions',
      'GET /api/chat/statuses?sessions=wxid_qin',
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

  it('reads a conversation kept again once a live message came to it, and only that one', async () => {
    account = 'a';
    serve();
    await requestsOf(() => { render(<App />); });
    await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择周舟' })); });
    await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择林沐' })); });
    const pages = (requests: string[]) => requests.filter(request => request.startsWith('GET /api/chat/messages'));
    // a message comes to 周舟, kept but not open: nothing is read for it until it is opened
    expect(pages(await requestsOf(() => streams.find(s => s.url.startsWith('/api/push/messages'))!.emit('message.new', { sessionId: 'wxid_zhou' })))).toEqual([]);
    expect(pages(await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择周舟' })); }))).toEqual([
      `GET /api/chat/messages?session-id=wxid_zhou&${PAGE}`,
    ]);
    expect(pages(await requestsOf(() => { fireEvent.click(screen.getByRole('button', { name: '选择林沐' })); }))).toEqual([]);
  });

  it('ends on the account switched back to when the answers of the one between come late', async () => {
    account = 'a';
    const server = serve(() => account === 'b');
    await requestsOf(() => { render(<App />); });
    await requestsOf(() => switchAccount('b'));
    await requestsOf(() => switchAccount('a'));
    expect(conversations()).toEqual(['林沐', '周舟']);
    // account b's list and messages come back now
    await requestsOf(() => server.release());
    expect(conversations()).toEqual(['林沐', '周舟']);
    expect(messages().length).toBeGreaterThan(0);
    expect(messages().every(text => text.startsWith('a 的消息'))).toBe(true);
  });

  it('shows the hits of the last search when an earlier one answers after it', async () => {
    account = 'a';
    const server = serve(url => url.includes('keyword=%E4%B8%80'));
    await requestsOf(() => { render(<App />); });
    const box = screen.getByRole('searchbox', { name: '搜索此会话的消息' });
    for (const keyword of ['一', '二']) {
      await requestsOf(() => {
        fireEvent.change(box, { target: { value: keyword } });
        fireEvent.click(screen.getByRole('button', { name: '搜索' }));
      });
    }
    expect(hits()).toHaveLength(1);
    await requestsOf(() => server.release());
    expect(hits()).toHaveLength(1);
    expect(hits()[0]).toContain('说到二');
  });

  it('keeps an older page of the account before out of the new one', async () => {
    account = 'a';
    long = true;
    const server = serve(url => url.includes('before='));
    await requestsOf(() => { render(<App />); });
    expect(messages()).toHaveLength(50);
    // scrolled to the top: the page before is asked for, and comes after the account changed
    expect(await requestsOf(() => { fireEvent.scroll(document.querySelector('.message-pane')!); })).toEqual([
      `GET /api/chat/messages?session-id=wxid_lin&limit=50&before=wxid_lin.51&parsed=true`,
    ]);
    long = false;
    await requestsOf(() => switchAccount('b'));
    await requestsOf(() => server.release());
    expect(conversations()).toEqual(['秦川']);
    expect(messages().length).toBeGreaterThan(0);
    expect(messages().every(text => text.startsWith('b 的消息'))).toBe(true);
  });
});

describe('requests of an open group (#1148)', () => {
  const senders = () => Array.from(document.querySelectorAll('.message-pane .message-sender'), name => name.textContent);
  const members = (requests: string[]) => requests.filter(request => request.startsWith('GET /api/group/members'));
  const live = (sessionId: string) => streams.find(s => s.url.startsWith('/api/push/messages'))!.emit('message.new', { sessionId });

  it('reads its members again once after a burst of its live messages, keeping the names shown meanwhile', async () => {
    account = 'g';
    serve();
    expect(members(await requestsOf(() => { render(<App />); }))).toEqual(['GET /api/group/members?chatroom-id=team@chatroom']);
    expect(senders()).toEqual(['小林']);

    const gathered = async () => {
      log = [];
      await act(async () => { await new Promise(resolve => setTimeout(resolve, LIVE_READ_MS)); });
      await act(async () => {});
      return log;
    };
    // a message of another conversation reads nothing for this one
    await requestsOf(() => live('other@chatroom'));
    expect(members(await gathered())).toEqual([]);

    // 新同学 joins and speaks, 林沐 renames themselves in the group: the burst reads the members once, after a while
    joined = true;
    expect(members(await requestsOf(() => { live('team@chatroom'); live('team@chatroom'); live('team@chatroom'); }))).toEqual([]);
    // the new message is shown before that: the names known stay meanwhile
    expect(senders()).toEqual(['小林', 'wxid_new']);
    expect(members(await gathered())).toEqual(['GET /api/group/members?chatroom-id=team@chatroom']);
    expect(senders()).toEqual(['林林', '新同学']);
  });
});

describe('requests of the pages that name people (#402)', () => {
  const open = (page: string) => () => { fireEvent.click(within(screen.getByRole('navigation', { name: '主导航' })).getByRole('button', { name: page })); };
  const contacts = (requests: string[]) => requests.filter(request => request.startsWith('GET /api/chat/contacts'));

  it('read the contact list once for the account, and again for another', async () => {
    account = 'a';
    serve();
    await requestsOf(() => { render(<App />); });
    // the footprint names people in its page and in its AI card: one request for both
    expect(contacts(await requestsOf(open('我的足迹')))).toEqual(['GET /api/chat/contacts?parsed=true']);
    expect(contacts(await requestsOf(open('通讯录')))).toEqual([]);
    // Moments names its posters from the whole table, read once too
    expect(contacts(await requestsOf(open('朋友圈')))).toEqual(['GET /api/chat/contacts']);
    expect(contacts(await requestsOf(open('我的足迹')))).toEqual([]);
    expect(contacts(await requestsOf(open('朋友圈')))).toEqual([]);
    // another account: Moments, open, names its posters from the new table at once, and the list is read again
    expect(contacts(await requestsOf(() => switchAccount('b')))).toEqual(['GET /api/chat/contacts']);
    expect(contacts(await requestsOf(open('通讯录')))).toEqual(['GET /api/chat/contacts?parsed=true']);
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['秦川']);
  });
});
