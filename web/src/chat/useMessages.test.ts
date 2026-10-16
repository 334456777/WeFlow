import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { emitLive } from '../api/live';
import { forgetMessages, useMessages, type Jump } from './useMessages';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  forgetMessages();
});

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const NEWEST = 1_760_000_000;
const message = (total: number, n: number) =>
  ({ _cursor: 'c.' + n, localId: total - n, localType: 1, createTime: NEWEST - n * 60, isSend: 0, senderUsername: 'wxid_a', parsedContent: 'm' + n });

/**
 * A conversation of `total` messages a minute apart, served newest first in pages, with cursors `c.<n>` (0 is the
 * newest): the newest, `before` and `after` a cursor, or `since` a time.
 */
function serve(total: number) {
  const calls: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    const query = new URL(url, 'http://x').searchParams;
    const limit = Number(query.get('limit') || 50);
    const at = (key: string) => Number(query.get(key)!.split('.')[1]);
    // the newest message of the page, and the oldest one it may reach
    let first = 0;
    let last = total - 1;
    if (query.get('before')) first = at('before') + 1;
    else if (query.get('after')) { last = at('after') - 1; first = Math.max(0, last - limit + 1); }
    else if (query.get('since')) { last = Math.min(total - 1, Math.floor((NEWEST - Number(query.get('since'))) / 60)); first = Math.max(0, last - limit + 1); }
    const page = [];
    for (let n = first; n <= last && page.length < limit; n++) page.push(message(total, n));
    return json({ success: true, data: page });
  }));
  return calls;
}

describe('useMessages', () => {
  it('reads the newest page, then older pages before the oldest message shown', async () => {
    const calls = serve(120);
    const { result } = renderHook(() => useMessages('wxid_a'));
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    expect(calls[0]).toContain('parsed=true');
    expect(result.current.thread.messages).toHaveLength(50);
    // Oldest first, as they are shown.
    expect(result.current.thread.messages.at(-1)!.parsedContent).toBe('m0');
    expect(result.current.thread.hasMore).toBe(true);

    act(() => result.current.loadOlder());
    await waitFor(() => expect(result.current.thread.messages).toHaveLength(100));
    expect(calls[1]).toContain('before=c.49');
    expect(result.current.thread.messages[0].parsedContent).toBe('m99');
    act(() => result.current.loadOlder());
    await waitFor(() => expect(result.current.thread.messages).toHaveLength(120));
    expect(result.current.thread.hasMore).toBe(false);
    act(() => result.current.loadOlder());
    expect(calls).toHaveLength(3);
  });

  it('shows a conversation read before at once, and never the previous one after a switch', async () => {
    const calls = serve(10);
    const { result, rerender } = renderHook(({ id }) => useMessages(id), { initialProps: { id: 'wxid_a' } });
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    rerender({ id: 'wxid_b' });
    // The first render after the switch already belongs to the new conversation.
    expect(result.current.thread.status).toBe('loading');
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    rerender({ id: 'wxid_a' });
    expect(result.current.thread.status).toBe('ready');
    expect(calls).toHaveLength(2);
  });

  it('jumps to a day: its first message on top, newer pages below, and back to the newest', async () => {
    const calls = serve(300);
    // the day of message 200: the page is messages 200 down to 151, oldest first
    const jump: Jump = { sessionId: 'wxid_a', target: { at: 'time', seconds: NEWEST - 200 * 60 }, seq: 1 };
    const { result } = renderHook(() => useMessages('wxid_a', jump));
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    expect(calls[0]).toContain('since=' + (NEWEST - 200 * 60));
    const thread = result.current.thread;
    expect(thread.messages[0].parsedContent).toBe('m200');
    expect(thread.messages.at(-1)!.parsedContent).toBe('m151');
    expect(thread.focus).toEqual({ cursor: 'c.200', align: 'start', seq: 1 });
    expect(thread.hasNewer).toBe(true);
    expect(thread.hasMore).toBe(true);

    act(() => result.current.loadNewer());
    await waitFor(() => expect(result.current.thread.messages).toHaveLength(100));
    expect(calls[1]).toContain('after=c.151');
    expect(result.current.thread.messages.at(-1)!.parsedContent).toBe('m101');
    act(() => result.current.loadOlder());
    await waitFor(() => expect(result.current.thread.messages).toHaveLength(150));
    expect(result.current.thread.messages[0].parsedContent).toBe('m250');

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.thread.messages.at(-1)?.parsedContent).toBe('m0'));
    expect(result.current.thread.hasNewer).toBe(false);
    expect(result.current.thread.focus).toBeNull();
  });

  it('opens a search hit in the middle of the messages around it', async () => {
    const calls = serve(300);
    const hit = message(300, 120);
    const jump: Jump = { sessionId: 'wxid_b', target: { at: 'message', message: hit }, seq: 7 };
    const { result, rerender } = renderHook(({ id, to }) => useMessages(id, to), { initialProps: { id: 'wxid_a', to: null as Jump | null } });
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    // switching to the hit's conversation shows it loading, never its newest page first
    rerender({ id: 'wxid_b', to: jump });
    expect(result.current.thread.status).toBe('loading');
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    expect(calls.slice(1).some(c => c.includes('before=c.120') && c.includes('limit=25'))).toBe(true);
    expect(calls.slice(1).some(c => c.includes('after=c.120') && c.includes('limit=25'))).toBe(true);
    const texts = result.current.thread.messages.map(m => m.parsedContent);
    expect(texts).toHaveLength(51);
    expect(texts[25]).toBe('m120');
    expect(texts[0]).toBe('m145');
    expect(texts[50]).toBe('m95');
    expect(result.current.thread.focus).toEqual({ cursor: 'c.120', align: 'center', seq: 7 });
    // a thread left in the past is not kept: back in the conversation, it opens at the newest again
    rerender({ id: 'wxid_a', to: jump });
    rerender({ id: 'wxid_b', to: jump });
    await waitFor(() => expect(result.current.thread.messages.at(-1)?.parsedContent).toBe('m0'));
  });

  it('asks for a newer weflow when the server does not know --parsed', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: false, error: { code: 'invalid_arguments', message: 'unexpected argument --parsed' } }, 400)));
    const { result } = renderHook(() => useMessages('wxid_a'));
    await waitFor(() => expect(result.current.thread.status).toBe('error'));
    expect(result.current.thread.error).toContain('chat messages --parsed');
  });
});

describe('live messages (#201)', () => {
  /** A conversation whose newest message is `last` (cursor `c.<n>`, older ones lower), with `recalled` ones changed. */
  function live(state: { last: number; recalled: Set<number> }) {
    const calls: string[] = [];
    const said = (n: number) => ({ ...message(1000, 1000 - n), _cursor: 'c.' + n, localId: n, parsedContent: state.recalled.has(n) ? '撤回了一条消息' : 'm' + n });
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      const query = new URL(url, 'http://x').searchParams;
      const limit = Number(query.get('limit') || 50);
      const after = query.get('after');
      const from = after ? Number(after.split('.')[1]) + 1 : Math.max(0, state.last - limit + 1);
      const page = [];
      // newest first; after a cursor, the closest ones
      const top = after ? Math.min(state.last, from + limit - 1) : state.last;
      for (let n = top; n >= from; n--) page.push(said(n));
      return json({ success: true, data: page });
    }));
    return calls;
  }
  const texts = (thread: { messages: { parsedContent?: string | null }[] }) => thread.messages.map(m => m.parsedContent);

  it('adds the messages that came, all of them, to the open conversation only', async () => {
    const state = { last: 2, recalled: new Set<number>() };
    const calls = live(state);
    const { result } = renderHook(() => useMessages('wxid_a'));
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    expect(texts(result.current.thread)).toEqual(['m0', 'm1', 'm2']);
    // another conversation: nothing read
    act(() => emitLive({ type: 'message.new', sessionId: 'wxid_b' }));
    expect(calls).toHaveLength(1);
    // more than a page came at once
    state.last = 120;
    act(() => emitLive({ type: 'message.new', sessionId: 'wxid_a' }));
    await waitFor(() => expect(result.current.thread.messages).toHaveLength(121));
    expect(texts(result.current.thread).slice(-2)).toEqual(['m119', 'm120']);
    expect(calls.slice(1).every(url => url.includes('after='))).toBe(true);
    // twice for one message: once shown
    act(() => { emitLive({ type: 'message.new', sessionId: 'wxid_a' }); emitLive({ type: 'message.new', sessionId: 'wxid_a' }); });
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(6));
    expect(result.current.thread.messages).toHaveLength(121);
  });

  it('shows a recall in place, and catches up after a break', async () => {
    const state = { last: 3, recalled: new Set<number>() };
    live(state);
    const { result } = renderHook(() => useMessages('wxid_a'));
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    state.recalled.add(2);
    act(() => emitLive({ type: 'message.revoke', sessionId: 'wxid_a' }));
    await waitFor(() => expect(texts(result.current.thread)).toEqual(['m0', 'm1', '撤回了一条消息', 'm3']));
    state.last = 4;
    state.recalled.add(3);
    act(() => emitLive({ type: 'resumed' }));
    await waitFor(() => expect(texts(result.current.thread)).toEqual(['m0', 'm1', '撤回了一条消息', '撤回了一条消息', 'm4']));
  });

  it('leaves a conversation left in the past by a jump alone', async () => {
    const state = { last: 200, recalled: new Set<number>() };
    const calls = live(state);
    const hit = { _cursor: 'c.100', _session_id: 'wxid_a', localId: 100, localType: 1, createTime: 1, isSend: 0, senderUsername: 'wxid_a', parsedContent: 'm100' };
    const jump: Jump = { sessionId: 'wxid_a', target: { at: 'message', message: hit as never }, seq: 1 };
    const { result } = renderHook(() => useMessages('wxid_a', jump));
    await waitFor(() => expect(result.current.thread.status).toBe('ready'));
    expect(result.current.thread.hasNewer).toBe(true);
    const read = calls.length;
    act(() => emitLive({ type: 'message.new', sessionId: 'wxid_a' }));
    expect(calls).toHaveLength(read);
  });
});
