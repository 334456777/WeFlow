import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { loadMembers, memberName } from '../chat/LiveMessages';
import { forgetMessages, useMessages } from '../chat/useMessages';
import { useDateCounts } from '../chat/useDateCounts';
import type { Backend } from './backend';
import { currentScope, newScope, useScope } from './scope';

afterEach(() => {
  vi.unstubAllGlobals();
  forgetMessages();
});

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
const said = (text: string) => ({ _cursor: 'c.0', localId: 1, localType: 1, createTime: 1_760_000_000, isSend: 0, senderUsername: 'filehelper', parsedContent: text });

/** A server holding one account's records: `filehelper` says the account's name; `calls` counts the reads. */
function account(name: string, calls: string[], gate?: Promise<void>) {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    await gate;
    if (url.includes('group/members')) return json({ success: true, data: { members: [{ username: 'wxid_m', displayName: 'member of ' + name }] } });
    if (url.includes('date-counts')) return json({ success: true, data: { counts: { [name === 'A' ? '2026-10-01' : '2026-10-02']: 1 } } });
    return json({ success: true, data: [said('account ' + name)] });
  }));
}

const ready = (data?: string): Backend => ({ state: 'ready', health: { status: 'ok', apiVersion: '2', version: '5.0.0', data } });

describe('scopes (#198)', () => {
  it('starts a new one when the data changes or the server comes back', () => {
    const { result, rerender } = renderHook(({ backend }) => useScope(backend), { initialProps: { backend: { state: 'checking' } as Backend } });
    const start = result.current;
    rerender({ backend: ready('a') });
    rerender({ backend: ready('a') });
    expect(result.current).toBe(start);
    // the server restarted, or the account changed, while the page was connected
    rerender({ backend: ready('b') });
    expect(result.current).toBe(start + 1);
    // gone and back (an older server says no data)
    rerender({ backend: { state: 'offline' } });
    rerender({ backend: ready('b') });
    expect(result.current).toBe(start + 2);
    rerender({ backend: ready(undefined) });
    expect(result.current).toBe(start + 3);
    rerender({ backend: { state: 'offline' } });
    rerender({ backend: ready(undefined) });
    expect(result.current).toBe(start + 4);
  });

  it('never shows the account before for the same conversation', async () => {
    const calls: string[] = [];
    account('A', calls);
    const first = renderHook(() => useMessages('filehelper'));
    await waitFor(() => expect(first.result.current.thread.messages[0]?.parsedContent).toBe('account A'));
    first.unmount();

    // without a new scope, the conversation is kept (as when switching conversations)
    const again = renderHook(() => useMessages('filehelper'));
    expect(again.result.current.thread.messages[0]?.parsedContent).toBe('account A');
    again.unmount();
    expect(calls).toHaveLength(1);

    account('B', calls);
    newScope();
    const second = renderHook(() => useMessages('filehelper'));
    expect(second.result.current.thread.messages).toHaveLength(0);
    await waitFor(() => expect(second.result.current.thread.messages[0]?.parsedContent).toBe('account B'));
    expect(calls).toHaveLength(2);
  });

  it('does not keep an answer to a request of the scope before', async () => {
    const calls: string[] = [];
    let open!: () => void;
    account('A', calls, new Promise(resolve => { open = resolve; }));
    const reading = renderHook(() => useMessages('filehelper'));
    const members = loadMembers('g@chatroom');
    const counts = renderHook(() => useDateCounts('filehelper'));
    counts.result.current.request();
    await waitFor(() => expect(calls).toHaveLength(3));
    const before = currentScope();
    newScope();
    open();
    await waitFor(() => expect(reading.result.current.thread.status).toBe('ready'));
    await members;
    reading.unmount();
    counts.unmount();
    expect(currentScope()).toBe(before + 1);
    expect(memberName('g@chatroom', 'wxid_m')).toBeUndefined();

    account('B', calls);
    const later = renderHook(() => useMessages('filehelper'));
    await waitFor(() => expect(later.result.current.thread.messages[0]?.parsedContent).toBe('account B'));
    const laterCounts = renderHook(() => useDateCounts('filehelper'));
    laterCounts.result.current.request();
    await waitFor(() => expect(laterCounts.result.current.counts).toEqual({ '2026-10-02': 1 }));
  });
});
