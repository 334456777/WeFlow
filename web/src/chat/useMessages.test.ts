import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { forgetMessages, useMessages } from './useMessages';

afterEach(() => {
  vi.unstubAllGlobals();
  forgetMessages();
});

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** A conversation of `total` messages, served newest first in pages, with cursors `c.<n>` (0 is the newest). */
function serve(total: number) {
  const calls: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    const query = new URL(url, 'http://x').searchParams;
    const start = query.get('before') ? Number(query.get('before')!.split('.')[1]) + 1 : 0;
    const page = Array.from({ length: Math.max(0, Math.min(50, total - start)) }, (_, i) => {
      const n = start + i;
      return { _cursor: 'c.' + n, localId: total - n, localType: 1, createTime: 1_760_000_000 - n * 60, isSend: 0, senderUsername: 'wxid_a', parsedContent: 'm' + n };
    });
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

  it('asks for a newer weflow when the server does not know --parsed', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: false, error: { code: 'invalid_arguments', message: 'unexpected argument --parsed' } }, 400)));
    const { result } = renderHook(() => useMessages('wxid_a'));
    await waitFor(() => expect(result.current.thread.status).toBe('error'));
    expect(result.current.thread.error).toContain('chat messages --parsed');
  });
});
