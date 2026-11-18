import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { getBizAccounts } from '../api/biz';
import { emitLive, LIVE_READ_MS } from '../api/live';
import { BizPushes } from './BizPushes';
import { sessionRows, withOfficials } from './sessions';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
const session = { username: 'gh_news', displayName: '每日新闻', summary: '', unreadCount: 0, sortTimestamp: 0, lastTimestamp: 0, lastMsgType: 49 };
const push = (n: number, extra: object = {}) => ({
  localId: n, createTime: 1_760_000_000 - n * 3600, title: '标题 ' + n, description: '摘要 ' + n, url: 'https://mp.example/' + n, cover: '',
  contentList: [], ...extra,
});

describe('BizPushes (#203)', () => {
  it('shows pushes as article cards, newest first, and pages on', async () => {
    const calls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      const offset = Number(new URL(url, 'http://x').searchParams.get('offset'));
      const pushes = offset === 0
        ? [push(0, { contentList: [
          { title: '标题 0', url: 'https://mp.example/0', cover: '', summary: '' },
          { title: '第二篇', url: 'https://mp.example/0b', cover: '', summary: '' },
        ] }), ...Array.from({ length: 19 }, (_, i) => push(i + 1))]
        : [push(20)];
      return json({ success: true, data: { username: 'gh_news', messages: pushes } });
    }));
    render(<BizPushes session={session} />);
    await screen.findByRole('heading', { name: '每日新闻' });
    expect(calls[0]).toContain('/api/biz/messages?username=gh_news&limit=20&offset=0');
    const cards = document.querySelectorAll('.biz-push');
    expect(cards).toHaveLength(20);
    // the lead links to its article; the other article of the push follows as a row, the lead not twice
    expect(screen.getByRole('link', { name: '标题 0' }).getAttribute('href')).toBe('https://mp.example/0');
    expect(screen.getByRole('link', { name: '第二篇' }).getAttribute('target')).toBe('_blank');
    expect(cards[0].querySelectorAll('.biz-item')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: '加载更早的推送' }));
    await waitFor(() => expect(document.querySelectorAll('.biz-push')).toHaveLength(21));
    expect(calls[1]).toContain('offset=20');
    expect(screen.queryByRole('button', { name: '加载更早的推送' })).toBeNull();
  });

  it('says why there is nothing, and reads again on a live push', async () => {
    vi.useFakeTimers();
    let pushes: object[] = [];
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { messages: pushes } })));
    render(<BizPushes session={session} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(screen.getByText(/本机没有这个公众号的推送/)).toBeTruthy();
    pushes = [push(1)];
    act(() => emitLive({ type: 'message.new', sessionId: 'gh_news' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_READ_MS); });
    expect(screen.getByRole('link', { name: '标题 1' })).toBeTruthy();
  });

  /** An account whose pushes are `state.pushes` (`push(n)`, newest first); a page at `state.hold` stays on its way. */
  function serveFeed(state: { pushes: number[]; hold?: number }) {
    const calls: { offset: number; signal?: AbortSignal }[] = [];
    vi.stubGlobal('fetch', vi.fn((url: string, init: RequestInit = {}) => {
      const offset = Number(new URL(url, 'http://x').searchParams.get('offset'));
      const signal = init.signal ?? undefined;
      calls.push({ offset, signal });
      if (offset === state.hold) return new Promise((_resolve, reject) => signal?.addEventListener('abort', () => reject(signal.reason), { once: true }));
      return Promise.resolve(json({ success: true, data: { username: 'gh_news', messages: state.pushes.slice(offset, offset + 20).map(n => push(n)) } }));
    }));
    return calls;
  }
  const titles = () => Array.from(document.querySelectorAll('.biz-lead-title'), title => title.textContent);
  const settle = (ms = 0) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

  it('reads the newest again once for a burst of live pushes, keeping the older ones shown (#201)', async () => {
    vi.useFakeTimers();
    const state = { pushes: Array.from({ length: 45 }, (_, i) => i + 1) };
    const calls = serveFeed(state);
    render(<BizPushes session={session} />);
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '加载更早的推送' }));
    await settle();
    expect(titles()).toHaveLength(40);

    state.pushes = [0, ...state.pushes];
    act(() => { for (let i = 0; i < 3; i++) emitLive({ type: 'message.new', sessionId: 'gh_news' }); });
    emitLive({ type: 'message.new', sessionId: 'gh_other' });
    await settle(LIVE_READ_MS - 1);
    expect(calls).toHaveLength(2);
    // the feed stays while it reads: no skeleton
    expect(titles()).toHaveLength(40);
    await settle(1);
    expect(calls.map(c => c.offset)).toEqual([0, 20, 0]);
    expect(titles()).toHaveLength(41);
    expect(titles()[0]).toBe('标题 0');
    expect(new Set(titles()).size).toBe(41);

    // the next page goes on from where the feed ends
    fireEvent.click(screen.getByRole('button', { name: '加载更早的推送' }));
    await settle();
    expect(calls[3].offset).toBe(41);
    expect(titles()).toHaveLength(46);
    expect(titles().at(-1)).toBe('标题 45');
  });

  it('lets an older page on its way go when the newest is read again', async () => {
    vi.useFakeTimers();
    const state = { pushes: Array.from({ length: 45 }, (_, i) => i + 1), hold: 20 };
    const calls = serveFeed(state);
    render(<BizPushes session={session} />);
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '加载更早的推送' }));
    await settle();
    expect(screen.getByRole('button', { name: /正在加载/ })).toBeTruthy();

    state.pushes = [0, ...state.pushes];
    act(() => emitLive({ type: 'message.new', sessionId: 'gh_news' }));
    await settle(LIVE_READ_MS);
    expect(calls[1].signal?.aborted).toBe(true);
    // the new push on top of the 20 shown
    expect(titles()).toHaveLength(21);
    expect(titles()[0]).toBe('标题 0');
    expect(titles().at(-1)).toBe('标题 20');
    expect(screen.getByRole<HTMLButtonElement>('button', { name: '加载更早的推送' }).disabled).toBe(false);
  });

  it('shows an article whose address is not a web page without a link (#818)', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { messages: [push(1, { url: 'javascript:alert(1)' })] } })));
    render(<BizPushes session={session} />);
    expect(await screen.findByText('标题 1')).toBeTruthy();
    expect(screen.queryByRole('link')).toBeNull();
  });
});

describe('official accounts in the list (#203)', () => {
  it('reads the accounts followed from their contact rows', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { accounts: [
      { username: 'gh_a', nick_name: '甲', remark: '', big_head_url: 'https://wx.example/a.jpg', lastTime: 1_760_000_000 },
      { username: 'gh_b', alias: 'b', lastTime: null },
      { nick_name: 'no name' },
    ] } })));
    expect(await getBizAccounts()).toEqual([
      { username: 'gh_a', name: '甲', avatarUrl: 'https://wx.example/a.jpg', lastTime: 1_760_000_000 },
      { username: 'gh_b', name: 'b', avatarUrl: undefined, lastTime: 0 },
    ]);
  });

  it('adds the ones without a conversation under 「公众号」 only, and finds them by name', () => {
    const listed = [{ ...session }, { ...session, username: 'wxid_x', displayName: '林沐', sortTimestamp: 1_760_000_000 }];
    const officials = [{ ...session }, { ...session, username: 'gh_more', displayName: '城市手记' }];
    expect(withOfficials(listed, officials).map(s => s.username)).toEqual(['gh_news', 'wxid_x', 'gh_more']);
    expect(sessionRows(listed, {}, 'official', '', false, officials).map(r => r.id)).toEqual(['gh_news', 'gh_more']);
    expect(sessionRows(listed, {}, 'all', '', false, officials).map(r => r.id)).toEqual(['gh_news', 'wxid_x']);
    expect(sessionRows(listed, {}, 'official', '城市', false, officials).map(r => r.id)).toEqual(['gh_more']);
  });
});
