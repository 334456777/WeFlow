import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import { App } from './App';
import { setToken } from './api/client';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
});

/** The server's event stream (`/health/events`): `hello` with `serving`, or an error when there is no server. */
let serving: object | null = null;
const opened: FakeEventSource[] = [];

class FakeEventSource {
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((event: MessageEvent) => void)[]>();
  constructor(readonly url: string) {
    opened.push(this);
    queueMicrotask(() => {
      // a job's events come only when a test sends them
      if (this.closed || url.startsWith('/jobs/')) return;
      if (serving) this.emit('hello', serving);
      else this.fail();
    });
  }
  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() { this.closed = true; }
  emit(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(new MessageEvent(type, { data: JSON.stringify(data) }));
  }
  fail() { this.onerror?.(new Event('error')); }
}

/** Runs a server saying `health` (none for null). */
function serve(health: object | null) {
  serving = health;
  vi.stubGlobal('EventSource', FakeEventSource);
}

async function renderWith(health: object | null) {
  serve(health);
  render(<App />);
  await act(async () => {});
}

const json = (body: unknown) => new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } });
const HEALTH = { status: 'ok', apiVersion: '1', version: '5.0.0-rc.6', data: 'a' };

describe('App', () => {
  it('shows the workspace with the sample records while no backend is connected', async () => {
    await renderWith(null);
    expect(screen.getByRole('navigation', { name: '主导航' })).toBeTruthy();
    expect(screen.getByRole('region', { name: '操作控制台' })).toBeTruthy();
    expect(screen.queryByText('操作控制台')).toBeNull();
    expect(Array.from(document.querySelectorAll('.card-title'), title => title.textContent)).toEqual(['导出', '任务中心', '搜索', '会话资料', '媒体文件']);
    expect(screen.getByText('示例数据 · 连接 weflow serve 后显示真实会话')).toBeTruthy();
    expect(screen.getByRole('button', { name: '选择设计协作群' }).textContent).toContain('李婷：这版看起来不错');
  });

  it('keeps official accounts in the chat list rather than a sidebar entry of their own', async () => {
    await renderWith(null);
    const nav = screen.getByRole('navigation', { name: '主导航' });
    expect(Array.from(nav.querySelectorAll('.nav-label'), label => label.textContent)).toEqual(['聊天', '朋友圈', '微信支付', '通讯录', '聊天分析', '年度报告', '灵感信箱', '导出', '数据库备份', '设置']);
    fireEvent.click(screen.getByRole('button', { name: '公众号' }));
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['设计观察', '城市手记']);
  });

  it('lists the conversations of a running backend', async () => {
    const calls: string[] = [];
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      if (url.startsWith('/api/chat/sessions')) {
        return json({ success: true, data: [
          { username: 'wxid_a', displayName: '林沐', summary: '你好', unreadCount: 1, sortTimestamp: 1760000000, lastTimestamp: 1760000000, lastMsgType: 1, messageCountHint: 42 },
          { username: 'g@chatroom', displayName: '家人群', summary: '晚饭', unreadCount: 0, sortTimestamp: 1750000000, lastTimestamp: 1750000000, lastMsgType: 1 },
        ] });
      }
      if (url.startsWith('/chat/statuses')) return json({ success: true, data: { 'g@chatroom': { isFolded: true, isMuted: true } } });
      if (url.startsWith('/api/chat/messages')) {
        return json({ success: true, data: [
          { _cursor: 'c.0', localId: 3, localType: 25769803825, appMsgKind: 'file', createTime: 1760000000, isSend: 1, senderUsername: 'wxid_me', parsedContent: '[文件] 计划.pdf', fileName: '计划.pdf', fileSize: 2048 },
          { _cursor: 'c.1', localId: 2, localType: 10000, createTime: 1759999990, isSend: 0, senderUsername: null, parsedContent: '"林沐" 撤回了一条消息' },
          { _cursor: 'c.2', localId: 1, localType: 1, createTime: 1759999900, isSend: 0, senderUsername: 'wxid_a', parsedContent: '你好' },
        ] });
      }
      throw new TypeError('unexpected ' + url);
    }));
    render(<App />);
    await act(async () => {});
    // live messages are on unless turned off
    expect(screen.getByRole('status').textContent).toBe('实时');
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['林沐', '折叠的群聊']);
    expect(calls).toContain('/chat/statuses?usernames=wxid_a&usernames=g%40chatroom');
    // The first conversation is opened; its details and media are read only once their cards open (#200).
    expect(screen.getByRole('button', { name: '选择林沐' }).getAttribute('aria-pressed')).toBe('true');
    expect(Array.from(document.querySelectorAll('.card-title'), title => title.textContent)).toEqual(['导出', '任务中心', '搜索', '会话资料', '媒体文件']);
    expect(calls.filter(url => /detail|resources|group\//.test(url))).toEqual([]);
    // the open conversation can be exported on the server (#211)
    expect(screen.getByRole('button', { name: '开始导出' })).toHaveProperty('disabled', false);
    expect(document.querySelector('[data-card="info"] .card-summary')?.textContent).toBe('约 42 条');
    // Its messages, oldest first: a text, a notice in the middle, a file sent by me.
    await act(async () => {});
    const rows = Array.from(document.querySelectorAll('.message-pane .message-row'));
    expect(rows.map(row => row.className)).toEqual(['message-row incoming', 'message-row system', 'message-row outgoing']);
    expect(rows[0].textContent).toContain('你好');
    expect(rows[1].textContent).toBe('"林沐" 撤回了一条消息');
    expect(rows[2].textContent).toContain('计划.pdf');
    expect(rows[2].textContent).toContain('2.0 KB');
    expect(calls.find(url => url.startsWith('/api/chat/messages'))).toContain('session-id=wxid_a');
  });

  it('says why the conversations could not be read and tries again', async () => {
    let fail = true;
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url.startsWith('/api/chat/sessions')) {
        return fail
          ? new Response(JSON.stringify({ success: false, error: { code: 'config_error', message: 'no account' } }), { status: 409 })
          : json({ success: true, data: [] });
      }
      return json({ success: true, data: {} });
    }));
    render(<App />);
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toContain('weflow 还没有配置账号');
    fail = false;
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    await act(async () => {});
    expect(screen.getByText('这个账号还没有会话。')).toBeTruthy();
  });

  it('says when there is no backend or an incompatible one', async () => {
    await renderWith(null);
    expect(screen.getByRole('status').textContent).toBe('未连接');
    cleanup();
    await renderWith({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.3', data: 'a' });
    expect(screen.getByRole('status').textContent).toBe('版本不兼容');
  });

  it('switches between the conversations and the settings', async () => {
    await renderWith(null);
    fireEvent.click(screen.getByRole('button', { name: '选择陈屿' }));
    expect(screen.getByText('好的，周一见。', { selector: '.message-bubble' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(screen.getByText('weflow serve')).toBeTruthy();
  });

  it('exports the open conversation as a job of the server: progress, cancel, then the file to save (#211)', async () => {
    const job = (id: string, status: string, more: object = {}) =>
      ({ id, status, command: ['weflow', '--json', '--progress', 'export', 'messages'], createdAt: 1760000000000, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null, file: null, ...more });
    let jobs: ReturnType<typeof job>[] = [];
    const started: { url: string; init?: RequestInit }[] = [];
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (url.startsWith('/api/chat/sessions')) {
        return json({ success: true, data: [{ username: 'wxid_a', displayName: '林沐', summary: '你好', unreadCount: 0, sortTimestamp: 1760000000, lastTimestamp: 1760000000, lastMsgType: 1 }] });
      }
      if (url.startsWith('/export/messages')) {
        started.push({ url, init });
        const next = job('j' + started.length, 'running');
        jobs = [...jobs, next];
        return json({ success: true, data: next });
      }
      if (url === '/jobs') return json({ success: true, data: jobs });
      if (url.startsWith('/jobs/') && init?.method === 'DELETE') {
        const id = url.split('/')[2];
        jobs = jobs.map(j => (j.id === id ? { ...j, status: 'cancelled' } : j));
        return json({ success: true, data: jobs.find(j => j.id === id) });
      }
      return json({ success: true, data: [] });
    }));
    render(<App />);
    await act(async () => {});
    const side = () => screen.getByRole('button', { name: /取消导出|再次导出/, hidden: true });
    expect(side().hasAttribute('inert')).toBe(true);

    // any format of `export messages`, not only the first three (#279)
    fireEvent.click(within(document.querySelector<HTMLElement>('[data-card="export"]')!).getByRole('button', { name: 'TXT' }));
    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    await act(async () => {});
    // a job of the server that keeps its file
    expect(started[0].url).toBe('/export/messages?return=file');
    expect((started[0].init?.headers as Record<string, string>).Prefer).toBe('respond-async');
    expect(JSON.parse(String(started[0].init?.body))).toMatchObject({ 'session-id': 'wxid_a', format: 'txt', media: 'all' });
    expect(screen.getByRole('button', { name: '正在导出' }).getAttribute('aria-disabled')).toBe('true');
    expect(side().hasAttribute('inert')).toBe(false);
    // its progress from the job's events, in the task centre too
    const events = opened.find(source => source.url.startsWith('/jobs/j1/events'))!;
    act(() => { events.emit('progress', { type: 'progress', message: '正在导出消息', current: 5, total: 10, percent: 50 }); });
    expect(screen.getByRole('progressbar', { name: '导出 林沐 · TXT的进度' }).getAttribute('aria-valuenow')).toBe('50');

    fireEvent.click(screen.getByRole('button', { name: '取消导出' }));
    await act(async () => {});
    expect(screen.getByRole('button', { name: '开始导出' })).toBeTruthy();
    expect(side().hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    await act(async () => {});
    const done = job('j2', 'succeeded', { file: { name: '林沐.html', size: 2048 }, result: { success: true, data: { out: '/srv/林沐.html' } } });
    jobs = jobs.map(j => (j.id === 'j2' ? done : j));
    await act(async () => { opened.find(source => source.url.startsWith('/jobs/j2/events'))!.emit('result', done); });
    const links = screen.getAllByRole('link', { name: '下载文件' });
    expect(links).toHaveLength(2);
    expect(links[0].getAttribute('href')).toBe('/jobs/j2/file');
    expect(side().getAttribute('aria-label')).toBe('再次导出');
    expect(screen.getByText(/林沐\.html · 2\.0 KB/)).toBeTruthy();
    // a finished job is deleted, with its file on the server: the cancelled one and the one done
    const deletes = screen.getAllByRole('button', { name: '删除' });
    expect(deletes.map(button => button.getAttribute('title'))).toEqual(['删除这个任务和它在服务端的文件', '删除这个任务']);
  });

  it('lists the contacts by kind, searches them, shows one and opens its conversation, and exports them (#205)', async () => {
    const calls: string[] = [];
    const started: { url: string; body: Record<string, unknown> }[] = [];
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push(url);
      if (url.startsWith('/api/chat/sessions')) {
        return json({ success: true, data: [
          { username: 'wxid_a', displayName: '林沐', summary: '', unreadCount: 0, sortTimestamp: 3, lastTimestamp: 3, lastMsgType: 1 },
          { username: 'wxid_b', displayName: '陈屿', summary: '', unreadCount: 0, sortTimestamp: 2, lastTimestamp: 2, lastMsgType: 1 },
        ] });
      }
      if (url.startsWith('/api/chat/contacts')) {
        return json({ success: true, data: [
          { username: 'wxid_b', displayName: '小陈', remark: '小陈', nickname: '陈屿', alias: 'chenyu', region: '中国 上海', detailDescription: '慢慢走', labels: ['同学'], type: 'friend' },
          { username: 'g@chatroom', displayName: '家人群', type: 'group' },
          { username: 'wxid_c', displayName: '秋秋', nickname: '秋秋', type: 'friend' },
          { username: 'gh_x', displayName: '城市手记', detailDescription: '一条小街', type: 'official' },
        ] });
      }
      if (url.startsWith('/export/')) {
        started.push({ url, body: JSON.parse(String(init?.body)) });
        return json({ success: true, data: { id: 'c1', status: 'queued', command: ['weflow', 'export', 'contacts'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null } });
      }
      return json({ success: true, data: [] });
    }));
    render(<App />);
    await act(async () => {});
    expect(calls.some(url => url.startsWith('/api/chat/contacts'))).toBe(false);
    fireEvent.click(within(screen.getByRole('navigation', { name: '主导航' })).getByRole('button', { name: '通讯录' }));
    await act(async () => {});
    expect(calls.filter(url => url.startsWith('/api/chat/contacts'))).toEqual(['/api/chat/contacts?parsed=true']);
    expect(Array.from(document.querySelectorAll('.card-title'), t => t.textContent)).toEqual(['任务中心', '通讯录统计', '导出通讯录']);
    // the friends first, with how many of each kind
    const names = () => Array.from(document.querySelectorAll('.conversation-name'), n => n.textContent);
    expect(names()).toEqual(['小陈', '秋秋']);
    expect(screen.getByRole('button', { name: '群聊 1 个' })).toBeTruthy();
    expect(document.querySelector('.conversation-preview')?.textContent).toBe('陈屿 · 中国 上海');
    fireEvent.click(screen.getByRole('button', { name: '公众号 1 个' }));
    expect(names()).toEqual(['城市手记']);
    // a search goes through every kind
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索联系人' }), { target: { value: 'CHENYU' } });
    expect(names()).toEqual(['小陈']);

    fireEvent.click(screen.getByRole('button', { name: '查看小陈' }));
    const card = screen.getByRole('article', { name: '小陈的资料' });
    expect(card.textContent).toContain('微信号chenyu');
    expect(card.textContent).toContain('签名慢慢走');
    expect(card.textContent).toContain('同学');

    // only the friends and groups, as Excel
    fireEvent.click(within(document.querySelector<HTMLElement>('[data-card="contacts-export"]')!).getByRole('button', { name: '公众号' }));
    fireEvent.click(within(document.querySelector<HTMLElement>('[data-card="contacts-export"]')!).getByRole('button', { name: '曾经的好友' }));
    fireEvent.click(document.querySelector<HTMLButtonElement>('[data-card="contacts-export"] .primary-button')!);
    await act(async () => {});
    expect(started).toEqual([{ url: '/export/contacts?return=file', body: { format: 'excel', parsed: true, kind: ['friend', 'group'] } }]);
    expect(document.querySelector('.jobs .task strong')?.textContent).toBe('导出通讯录 · 好友、群聊 · Excel');

    // the conversation with them
    fireEvent.click(within(card).getByRole('button', { name: '查看聊天记录' }));
    await act(async () => {});
    expect(screen.getByRole('button', { name: '选择陈屿' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('exports the conversations picked on the export page, one job each, with the options chosen (#211)', async () => {
    const started: { url: string; body: Record<string, unknown> }[] = [];
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (url.startsWith('/api/chat/sessions')) {
        return json({ success: true, data: [
          { username: 'wxid_a', displayName: '林沐', summary: '', unreadCount: 0, sortTimestamp: 3, lastTimestamp: 3, lastMsgType: 1 },
          { username: 'g@chatroom', displayName: '家人群', summary: '', unreadCount: 0, sortTimestamp: 2, lastTimestamp: 2, lastMsgType: 1 },
          { username: 'g2@chatroom', displayName: '同学群', summary: '', unreadCount: 0, sortTimestamp: 1, lastTimestamp: 1, lastMsgType: 1 },
        ] });
      }
      if (url.startsWith('/export/')) {
        const body = JSON.parse(String(init?.body));
        started.push({ url, body });
        return json({ success: true, data: { id: 'j' + started.length, status: 'queued', command: ['weflow', 'export'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null } });
      }
      return json({ success: true, data: [] });
    }));
    render(<App />);
    await act(async () => {});
    fireEvent.click(within(screen.getByRole('navigation', { name: '主导航' })).getByRole('button', { name: '导出' }));
    await act(async () => {});
    expect(Array.from(document.querySelectorAll('.card-title'), t => t.textContent)).toEqual(['任务中心', '批量导出', '导出会话列表']);
    expect(screen.getByRole('button', { name: '先在左侧选择会话' })).toHaveProperty('disabled', true);
    // the groups, all at once, then one taken off again beside the list
    fireEvent.click(screen.getByRole('button', { name: '群聊' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '全选当前列出的会话' }));
    expect(screen.getByRole('heading', { name: '已选 2 个会话' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '不导出同学群' }));
    fireEvent.click(screen.getByRole('button', { name: '全部' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '导出林沐' }));
    // Excel with the compact columns, by remark
    fireEvent.click(screen.getByRole('button', { name: 'Excel' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Excel 精简列' }));
    fireEvent.click(screen.getByRole('button', { name: '备注' }));
    fireEvent.click(screen.getByRole('button', { name: '导出 2 个会话' }));
    await act(async () => {});
    expect(started.map(s => s.url)).toEqual(['/export/messages?return=file', '/export/messages?return=file']);
    expect(started.map(s => s.body['session-id'])).toEqual(['wxid_a', 'g@chatroom']);
    expect(started[0].body).toEqual({ 'session-id': 'wxid_a', format: 'excel', 'display-name': 'remark', 'excel-compact': true });
    expect(screen.getByText('已加入 2 个导出任务，在任务中心查看进度和下载文件。')).toBeTruthy();
    expect(Array.from(document.querySelectorAll('.jobs .task strong'), n => n.textContent)).toEqual(['导出 家人群 · Excel', '导出 林沐 · Excel']);
    // the list of the conversations picked
    fireEvent.click(document.querySelector<HTMLButtonElement>('[data-card="sessions-export"] .secondary-button')!);
    await act(async () => {});
    expect(started[2]).toEqual({ url: '/export/sessions?return=file', body: { session: ['wxid_a', 'g@chatroom'] } });
  });

  it('follows the reduce-motion switch on the root element', async () => {
    await renderWith(null);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '减少动画' }));
    expect(document.documentElement.classList.contains('reduced-motion')).toBe(true);
    fireEvent.click(screen.getByRole('checkbox', { name: '减少动画' }));
    expect(document.documentElement.classList.contains('reduced-motion')).toBe(false);
  });
  it('hides a steady connection after 3 s, shows it again when it is lost, and keeps showing no connection', async () => {
    vi.useFakeTimers();
    onTestFinished(() => { vi.useRealTimers(); });
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: [] })));
    render(<App />);
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    const status = () => document.querySelector('.connection')!;
    expect(status().textContent).toBe('实时');
    expect(status().classList.contains('quiet')).toBe(false);

    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(status().classList.contains('quiet')).toBe(true);

    // the server stops: the stream breaks, and the tries to connect again fail
    serving = null;
    await act(async () => { opened[opened.length - 1].fail(); });
    expect(status().textContent).toBe('未连接');
    expect(status().classList.contains('quiet')).toBe(false);
    await act(async () => { await vi.advanceTimersByTimeAsync(60000); });
    expect(status().classList.contains('quiet')).toBe(false);
  });

  it('shows live messages in the list and the open conversation, and stops them when turned off (#201)', async () => {
    let newest = 1760000000;
    let read = 0;
    const said = (n: number) => ({ _cursor: 'c.' + n, localId: n, localType: 1, createTime: 1760000000 + n, isSend: 0, senderUsername: 'wxid_a', parsedContent: 'm' + n });
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (url.startsWith('/api/chat/sessions')) {
        read++;
        return json({ success: true, data: [
          { username: 'wxid_b', displayName: '陈屿', summary: 'b', sortTimestamp: 1760000001, lastTimestamp: 1760000001, lastMsgType: 1 },
          { username: 'wxid_a', displayName: '林沐', summary: 'm' + (newest - 1760000000), sortTimestamp: newest, lastTimestamp: newest, lastMsgType: 1 },
        ].sort((x, y) => y.sortTimestamp - x.sortTimestamp) });
      }
      if (url.startsWith('/chat/statuses')) return json({ success: true, data: {} });
      if (url.startsWith('/api/chat/messages')) {
        const after = new URL(url, 'http://x').searchParams.get('after');
        const last = newest - 1760000000;
        if (after) {
          const from = Number(after.split('.')[1]);
          return json({ success: true, data: Array.from({ length: last - from }, (_, i) => said(last - i)) });
        }
        return json({ success: true, data: Array.from({ length: last + 1 }, (_, i) => said(last - i)).slice(0, 50) });
      }
      return json({ success: true, data: null });
    }));
    render(<App />);
    await act(async () => {});
    const stream = opened[opened.length - 1];
    expect(stream.url).toContain('/api/push/messages');
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['陈屿', '林沐']);
    // 林沐 writes twice: the list puts the conversation on top, once for both
    newest += 2;
    const before = read;
    await act(async () => {
      stream.emit('message.new', { sessionId: 'wxid_a' });
      stream.emit('message.new', { sessionId: 'wxid_a' });
      await new Promise(resolve => setTimeout(resolve, 700));
    });
    expect(read).toBe(before + 1);
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['林沐', '陈屿']);
    // open, it shows them; one more comes below the last
    fireEvent.click(screen.getByRole('button', { name: /选择林沐/ }));
    await act(async () => {});
    const texts = () => Array.from(document.querySelectorAll('.message-pane .message-row .message-body'), body => body.textContent);
    expect(texts()).toEqual(['m0', 'm1', 'm2']);
    newest += 1;
    await act(async () => { stream.emit('message.new', { sessionId: 'wxid_a' }); await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(texts()).toEqual(['m0', 'm1', 'm2', 'm3']);

    // turned off: the state alone, from /health/events
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '实时消息' }));
    await act(async () => {});
    expect(opened[opened.length - 1].url).toBe('/health/events');
    expect(stream.closed).toBe(true);
    expect(screen.getByRole('status').textContent).toBe('已连接');
    expect(localStorage.getItem('weflow.live')).toBe('off');
  });

  it('asks for the token when the server refuses it, and goes on with the one typed (#214)', async () => {
    let accepted = 'right';
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      const bearer = new Headers(init?.headers).get('Authorization');
      if (bearer !== 'Bearer ' + accepted) return new Response(JSON.stringify({ success: false, error: { code: 'unauthorized', message: 'invalid or missing access token' } }), { status: 401 });
      if (url.startsWith('/api/chat/sessions')) return json({ success: true, data: [{ username: 'wxid_a', displayName: '林沐', summary: '你好', sortTimestamp: 1760000000, lastTimestamp: 1760000000, lastMsgType: 1 }] });
      return json({ success: true, data: url.startsWith('/api/chat/messages') ? [] : {} });
    }));
    render(<App />);
    await act(async () => {});
    await act(async () => {});
    expect(screen.getByRole('status').textContent).toBe('需要令牌');
    expect(screen.getByRole('alert').textContent).toContain('?token=');
    // typed, kept for this tab only, never shown again
    fireEvent.change(screen.getByLabelText('令牌'), { target: { value: ' right ' } });
    fireEvent.click(screen.getByRole('button', { name: '连接' }));
    expect(sessionStorage.getItem('weflow.token')).toBe('right');
    expect(localStorage.getItem('weflow.token')).toBeNull();
    expect((screen.queryByLabelText('令牌') as HTMLInputElement | null)?.value ?? '').toBe('');
    await act(async () => {});
    await act(async () => {});
    expect(opened[opened.length - 1].url).toContain('access_token=right');
    expect(screen.getByRole('status').textContent).toBe('实时');
    expect(Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent)).toEqual(['林沐']);
    setToken(null);
  });

  it('says in the settings what the server is, whether it suits the page, and what to do when not (#214)', async () => {
    await renderWith({ ...HEALTH, limits: { maxTasks: 1, maxReads: 4, maxMedia: 4 }, insight: true });
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const text = () => screen.getByRole('region', { name: '设置' }).textContent ?? '';
    expect(text()).toContain('weflow 5.0.0-rc.6 · API 1');
    expect(text()).toContain('适用于这个网页');
    expect(text()).toContain('同时运行任务 1 个 · 读取 4 个 · 媒体解码 4 个 · 灵感引擎已开启');
    expect(text()).toContain('没有保存令牌');
    cleanup();
    await renderWith({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.3', data: 'a' });
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(text()).toContain('不适用于这个网页');
    expect(text()).toContain('weflow web use');
  });

  it('opens the account setup by itself when the account is not set up, once a tab, and from the settings (#216)', async () => {
    let complete = false;
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === '/config/profiles') return json({ success: true, data: [{ name: 'default', current: true, wxid: complete ? 'wxid_a' : null, dbPath: complete ? 'D:/x' : null, decryptKey: complete, imageKeys: false, complete }] });
      if (url === '/db/detect') return json({ success: true, data: { candidates: [] } });
      return json({ success: true, data: [] });
    }));
    await renderWith(HEALTH);
    const dialog = () => document.querySelector<HTMLElement>('dialog.setup-dialog');
    expect(dialog()?.textContent).toContain('数据库目录');
    expect(document.querySelector('.setup-step[aria-current="step"]')?.textContent).toBe('1数据库目录');
    fireEvent.click(screen.getByRole('button', { name: '关闭配置向导', hidden: true }));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 400)); });
    expect(dialog()?.textContent).toBe('');
    expect(sessionStorage.getItem('weflow.setup')).toBe('dismissed');

    // the settings say how far the account is set up, and open the setup again
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(screen.getByRole('region', { name: '账号' }).textContent).toContain('default当前未选择账号还没有配置');
    fireEvent.click(screen.getByRole('button', { name: '配置向导' }));
    await act(async () => {});
    expect(dialog()?.textContent).toContain('数据库目录');

    // a tab that finds the account set up does not open it
    cleanup();
    sessionStorage.clear();
    complete = true;
    await renderWith(HEALTH);
    expect(dialog()?.textContent).toBe('');
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(screen.getByRole('region', { name: '账号' }).textContent).toContain('default当前wxid_aD:/x已配置，没有图片密钥');
  });

  it('shows the Moments of a running backend: who posts, their posts, and the cards of the page (#202)', async () => {
    const asked: string[] = [];
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      asked.push(url);
      if (url.startsWith('/api/sns/post-counts')) return json({ success: true, data: { wxid_a: 2, wxid_b: 1 } });
      if (url.startsWith('/api/chat/contacts')) return json({ success: true, data: [{ username: 'wxid_a', remark: '林沐' }, { username: 'wxid_b', nick_name: '小周' }] });
      if (url.startsWith('/api/sns/timeline')) {
        const only = new URL(url, 'http://x').searchParams.get('user');
        const posts = [
          { id: '1', username: 'wxid_a', nickname: '林沐', createTime: 1760000000, type: 2, contentDesc: '海边' },
          { id: '2', username: 'wxid_b', nickname: '小周', createTime: 1759990000, type: 2, contentDesc: '加班' },
        ].filter(p => !only || p.username === only);
        return json({ success: true, data: { timeline: posts } });
      }
      return json({ success: true, data: [] });
    }));
    render(<App />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '朋友圈' }));
    await act(async () => {});
    const people = screen.getByRole('region', { name: '朋友圈浏览' });
    expect(Array.from(people.querySelectorAll('.conversation-name'), n => n.textContent)).toEqual(['全部朋友圈', '林沐', '小周']);
    expect(Array.from(document.querySelectorAll('.moment-text'), n => n.textContent)).toEqual(['海边', '加班']);
    // the console has the page's own cards
    expect(Array.from(document.querySelectorAll('.card-title'), t => t.textContent)).toEqual(['筛选朋友圈', '朋友圈统计']);
    // one poster
    fireEvent.click(screen.getByRole('button', { name: '只看小周，1 条' }));
    await act(async () => {});
    expect(asked.at(-1)).toContain('user=wxid_b');
    expect(Array.from(document.querySelectorAll('.moment-text'), n => n.textContent)).toEqual(['加班']);
  });

  it('shows the WeChat Pay records of a running backend: all pages, by payee, with their totals (#204)', async () => {
    const asked: string[] = [];
    const at = (day: number) => Math.floor(new Date(2026, 9, day, 9).getTime() / 1000);
    const record = (localId: number, title: string, description: string, merchantName: string, day: number) =>
      ({ localId, createTime: at(day), title, description, merchantName, merchantIcon: '', timestamp: at(day) });
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      asked.push(url);
      if (url.startsWith('/biz/pay-records')) {
        const offset = Number(new URL(url, 'http://x').searchParams.get('offset'));
        return json({ success: true, data: offset === 0
          ? { records: [record(3, '微信支付凭证', '付款金额¥28.00\n支付方式零钱\n交易状态支付成功', '街角咖啡', 12)], hasMore: true }
          : { records: [record(2, '退款到账', '退款金额¥5.00\n退款方式零钱', '街角咖啡', 11), record(1, '微信支付凭证', '付款金额：¥12.50', '出行服务', 2)], hasMore: false } });
      }
      return json({ success: true, data: [] });
    }));
    render(<App />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '微信支付' }));
    await act(async () => {});
    // both pages were read
    expect(asked.filter(u => u.startsWith('/biz/pay-records'))).toHaveLength(2);
    const list = screen.getByRole('region', { name: '微信支付浏览' });
    expect(Array.from(list.querySelectorAll('.conversation-name'), n => n.textContent)).toEqual(['全部商户', '街角咖啡', '出行服务']);
    expect(Array.from(document.querySelectorAll('.pay-amount'), n => n.textContent)).toEqual(['-28.00', '+5.00', '-12.50']);
    expect(Array.from(document.querySelectorAll('.card-title'), t => t.textContent)).toEqual(['筛选支付记录', '合计', '导出支付记录']);
    // one payee: the refund is taken off
    fireEvent.click(screen.getByRole('button', { name: '只看街角咖啡，2 笔' }));
    expect(Array.from(document.querySelectorAll('.pay-amount'), n => n.textContent)).toEqual(['-28.00', '+5.00']);
    expect(within(list).getByRole('button', { name: '只看街角咖啡，2 笔' }).textContent).toContain('¥23.00');
  });

  it('switches the colour theme from the settings and remembers it (#252)', async () => {
    await renderWith(HEALTH);
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    const themes = screen.getByRole('group', { name: '颜色主题' });
    expect(within(themes).getByRole('button', { name: '跟随系统' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(within(themes).getByRole('button', { name: '深色' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(localStorage.getItem('weflow.theme')).toBe('dark');
    fireEvent.click(within(themes).getByRole('button', { name: '跟随系统' }));
    expect(localStorage.getItem('weflow.theme')).toBeNull();
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('shows the details, members and media of a real conversation once their cards open (#200)', async () => {
    const calls: string[] = [];
    const members = Array.from({ length: 1500 }, (_, i) => ({ username: 'wxid_m' + i, displayName: '成员' + i, groupNickname: i === 8 ? '小七' : '', isOwner: i === 0, isFriend: i % 2 === 0 }));
    serve(HEALTH);
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      const path = url.split('?')[0];
      if (path === '/api/chat/sessions') return json({ success: true, data: [{ username: 'g@chatroom', displayName: '家人群', summary: '晚饭', sortTimestamp: 1760000000, lastTimestamp: 1760000000, lastMsgType: 1 }] });
      if (path === '/chat/statuses') return json({ success: true, data: {} });
      if (path === '/api/chat/messages') {
        const around = url.includes('after=') || url.includes('before=');
        return json({ success: true, data: around ? [] : [{ _cursor: 'c.9', localId: 9, localType: 1, createTime: 1760000000, isSend: 0, senderUsername: 'wxid_m1', parsedContent: '晚饭' }] });
      }
      if (path === '/api/chat/detail') return json({ success: true, data: { wxid: 'g@chatroom', displayName: '家人群', messageCount: 12345 } });
      if (path === '/chat/detail') return json({ success: true, data: { firstMessageTime: 1600000000, latestMessageTime: 1760000000, messageTables: [] } });
      if (path === '/api/group/members') return json({ success: true, data: { members } });
      if (path === '/group/member') {
        return json({ success: true, data: { statistics: { totalMessages: 321, textMessages: 300, imageMessages: 21, voiceMessages: 0, videoMessages: 0, emojiMessages: 0, otherMessages: 0, firstMessageTime: 1600000000, lastMessageTime: 1760000000, activeDays: 42 }, timeDistribution: { 21: 50 }, commonPhrases: [{ phrase: '好的', count: 9 }], commonEmojis: [] } });
      }
      if (path === '/chat/resources') {
        return json({ success: true, data: { items: [{ localId: 5, createTime: 1750000000, localType: 25769803825, resourceType: 'file', fileName: '旅行计划.pdf', fileSize: 4096 }], total: 1, hasMore: false } });
      }
      if (path === '/api/chat/message') return json({ success: true, data: { _cursor: 'c.5', localId: 5, localType: 25769803825, appMsgKind: 'file', createTime: 1750000000, isSend: 1, senderUsername: 'wxid_me', parsedContent: '[文件] 旅行计划.pdf', fileName: '旅行计划.pdf', fileSize: 4096 } });
      return json({ success: true, data: null });
    }));
    render(<App />);
    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: /^会话资料/ }));
    await act(async () => {});
    const info = document.querySelector('[data-card="info"]')!;
    expect(info.textContent).toContain('12,345 条');
    expect(info.textContent).toContain('2020');
    expect(screen.getByRole('region', { name: '群成员' }).textContent).toContain('1500');
    // a thousand and more members: only those in sight are drawn
    expect(document.querySelectorAll('.member-row').length).toBeLessThan(60);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索群成员' }), { target: { value: '小七' } });
    expect(Array.from(document.querySelectorAll('.member-row'), row => row.getAttribute('aria-label'))).toEqual(['小七，好友']);
    fireEvent.click(screen.getByRole('button', { name: '小七，好友' }));
    await act(async () => {});
    expect(calls).toContain('/group/member?chatroom-id=g%40chatroom&username=wxid_m8');
    const stats = screen.getByRole('region', { name: '小七的群内统计' });
    expect(stats.textContent).toContain('321 条');
    expect(stats.textContent).toContain('21:00 – 22:00');
    expect(stats.textContent).toContain('好的');
    fireEvent.click(screen.getByRole('button', { name: '返回群成员' }));
    expect(screen.getByRole('region', { name: '群成员' })).toBeTruthy();

    // the media card: a file opens the conversation at its message
    fireEvent.click(screen.getByRole('button', { name: /^媒体文件/ }));
    await act(async () => {});
    expect(calls.find(url => url.startsWith('/chat/resources'))).toBe('/chat/resources?session=g%40chatroom&type=image&type=video&type=file&limit=60&offset=0');
    fireEvent.click(screen.getByRole('button', { name: /^旅行计划\.pdf/ }));
    await act(async () => {});
    expect(calls).toContain('/api/chat/message?session-id=g%40chatroom&local-id=5&create-time=1750000000&parsed=true');
    await act(async () => {});
    expect(document.querySelector('.message-pane')!.textContent).toContain('旅行计划.pdf');
  });

  it('folds the sidebar from its bottom-left button and remembers it', async () => {
    await renderWith(null);
    fireEvent.click(screen.getByRole('button', { name: '收起侧边栏' }));
    expect(screen.getByRole('main').classList.contains('sidebar-collapsed')).toBe(true);
    expect(localStorage.getItem('weflow.sidebar')).toBe('folded');
    cleanup();
    await renderWith(null);
    expect(screen.getByRole('main').classList.contains('sidebar-collapsed')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '展开侧边栏' }));
    expect(screen.getByRole('main').classList.contains('sidebar-collapsed')).toBe(false);
  });
});
