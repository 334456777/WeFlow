import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import { App } from './App';

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
      if (this.closed) return;
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
const HEALTH = { status: 'ok', apiVersion: '1', version: '5.0.0-rc.5', data: 'a' };

describe('App', () => {
  it('shows the workspace with the sample records while no backend is connected', async () => {
    await renderWith(null);
    expect(screen.getByRole('navigation', { name: '主导航' })).toBeTruthy();
    expect(screen.getByRole('region', { name: '操作控制台' })).toBeTruthy();
    expect(screen.queryByText('操作控制台')).toBeNull();
    expect(Array.from(document.querySelectorAll('.card-title'), title => title.textContent)).toEqual(['导出', '当前任务', '搜索', '会话资料', '媒体文件']);
    expect(screen.getByText('示例数据 · 连接 weflow serve 后显示真实会话')).toBeTruthy();
    expect(screen.getByRole('button', { name: '选择设计协作群' }).textContent).toContain('李婷：这版看起来不错');
  });

  it('keeps official accounts in the chat list rather than a sidebar entry of their own', async () => {
    await renderWith(null);
    const nav = screen.getByRole('navigation', { name: '主导航' });
    expect(Array.from(nav.querySelectorAll('.nav-label'), label => label.textContent)).toEqual(['聊天', '朋友圈', '微信支付', '设置']);
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
    // The first conversation is opened; its media wait for #200.
    expect(screen.getByRole('button', { name: '选择林沐' }).getAttribute('aria-pressed')).toBe('true');
    expect(Array.from(document.querySelectorAll('.card-title'), title => title.textContent)).toEqual(['导出', '当前任务', '搜索', '会话资料']);
    expect(screen.getByRole('button', { name: '开始导出' })).toHaveProperty('disabled', true);
    expect(screen.getByText('约 42 条记录', { exact: false })).toBeTruthy();
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

  it('stretches the export button into progress and cancel, and a cancelled export downloads nothing', async () => {
    // The HTML export waits for the photo; hold that fetch until the export has been cancelled.
    let releasePhoto = () => {};
    const photo = new Promise<void>(resolve => { releasePhoto = resolve; });
    serve(null);
    vi.stubGlobal('fetch', vi.fn(async () => {
      await photo;
      return new Response('<svg xmlns="http://www.w3.org/2000/svg"/>', { headers: { 'Content-Type': 'image/svg+xml' } });
    }));
    render(<App />);
    await act(async () => {});
    // jsdom has no object URLs; a download needs one.
    const createObjectURL = vi.fn(() => 'blob:export');
    Object.defineProperty(URL, 'createObjectURL', { value: createObjectURL, configurable: true });
    onTestFinished(() => { Reflect.deleteProperty(URL, 'createObjectURL'); });
    const side = () => screen.getByRole('button', { name: /取消导出|再次导出/, hidden: true });
    expect(side().hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    expect(screen.getByRole('button', { name: '正在导出' }).getAttribute('aria-disabled')).toBe('true');
    expect(side().hasAttribute('inert')).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: '取消导出' }));
    await act(async () => { releasePhoto(); await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(screen.getByRole('button', { name: '开始导出' })).toBeTruthy();
    expect(side().hasAttribute('inert')).toBe(true);
    expect(createObjectURL).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(screen.getAllByRole('link', { name: '下载文件' })).toHaveLength(2);
    expect(side().getAttribute('aria-label')).toBe('再次导出');
    expect(side().hasAttribute('inert')).toBe(false);
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
