import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useRef, useState } from 'react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { ChatSession } from '../api/chat';
import type { Resource } from '../api/details';
import type { Preview } from '../chat/MessageView';
import { ALL_RESOURCES, ResourceFilterCard, ResourcesPage } from './ResourcesPage';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }));
// 2025-10-03 and 2025-09-20, local time
const OCT = new Date(2025, 9, 3, 12).getTime() / 1000;
const SEP = new Date(2025, 8, 20, 12).getTime() / 1000;
const item = (n: number, resourceType: Resource['resourceType'], createTime: number, extra: Partial<Resource> = {}): Resource => ({
  _cursor: 'c' + n, localId: n, serverId: String(n), createTime, localType: 3, isSend: 0, senderUsername: 'wxid_a', content: '', text: null, resourceType, sessionId: 'wxid_a', sessionDisplayName: '林沐', ...extra,
});
const sessions = [
  { username: 'wxid_a', displayName: '林沐', sortTimestamp: OCT },
  { username: 'room@chatroom', displayName: '家人群', sortTimestamp: SEP },
] as ChatSession[];

function serve(pages: Record<string, unknown>) {
  const calls: string[] = [];
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    calls.push(url.pathname + url.search);
    if (url.pathname === '/api/chat/resources') return json(pages[url.searchParams.get('before') ?? ''] ?? { items: [], hasMore: false, nextCursor: null });
    // why a thumbnail failed
    return new Response(JSON.stringify({ success: false, error: { code: 'runtime_error', message: 'WXGF 图片无法转换：暂不支持它的格式（P and B slices (inter prediction)）', details: { failureKind: 'unsupported_image' } } }), { status: 422 });
  });
  vi.stubGlobal('fetch', fetch);
  return calls;
}

// the page in its scroll area and its console card, as the app puts them side by side
function Browser({ onPreview, onOpen }: { onPreview(p: Preview): void; onOpen(...args: unknown[]): void }) {
  const [filter, setFilter] = useState(ALL_RESOURCES);
  const scroller = useRef<HTMLElement>(null);
  return <><ResourceFilterCard sessions={sessions} filter={filter} onChange={setFilter} />
    <section ref={scroller} className="scroll-area"><ResourcesPage filter={filter} scroller={scroller} onPreview={onPreview} onOpen={onOpen} /></section></>;
}
const page = (onPreview = vi.fn(), onOpen = vi.fn()) => render(<Browser onPreview={onPreview} onOpen={onOpen} />);
const pick = (label: string) => fireEvent.click(within(screen.getByRole('group', { name: '资源类型' })).getByRole('button', { name: label }));

describe('the resource browser (#206)', () => {
  it('shows every type by month, and previews the pictures and videos read so far', async () => {
    serve({
      '': {
        items: [
          item(1, 'image', OCT),
          item(2, 'voice', OCT, { serverId: '77', content: '<msg><voicemsg voicelength="4200" /></msg>' }),
          item(3, 'video', SEP, { sessionId: 'room@chatroom', sessionDisplayName: '家人群' }),
          item(4, 'file', SEP, { fileName: '旅行计划.pdf', fileExt: 'pdf', fileSize: 4096 }),
        ],
        hasMore: false, nextCursor: null,
      },
    });
    const onPreview = vi.fn<(p: Preview) => void>();
    const onOpen = vi.fn();
    page(onPreview, onOpen);
    expect(await screen.findByRole('heading', { name: '2025年10月' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: '2025年9月' })).toBeTruthy();
    // a voice plays where it is, its length read from the message
    expect(screen.getByRole('button', { name: /播放语音，4″/ })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /^播放视频，家人群/ }));
    const preview = onPreview.mock.calls[0][0];
    expect(preview.type).toBe('video');
    if (preview.type === 'file') throw new Error();
    expect(preview.src).toContain('/api/chat/video-data?session-id=room%40chatroom&msg-id=3');
    expect(preview.gallery?.index).toBe(1);
    expect(preview.gallery?.items.map(p => p.type)).toEqual(['image', 'video']);
    preview.locate?.();
    expect(onOpen).toHaveBeenLastCalledWith('room@chatroom', 3, SEP);

    // a file opens its message
    fireEvent.click(screen.getByRole('button', { name: /^旅行计划\.pdf，林沐/ }));
    expect(onOpen).toHaveBeenLastCalledWith('wxid_a', 4, SEP);
  });

  it('narrows to a conversation and days, and reads on from the cursor of the page before', async () => {
    const calls = serve({
      '': { items: [item(1, 'image', OCT)], hasMore: true, nextCursor: 'k1' },
      k1: { items: [item(2, 'image', SEP)], hasMore: false, nextCursor: null },
    });
    page();
    await screen.findByRole('heading', { name: '2025年10月' });
    pick('图片');
    await waitFor(() => expect(calls[1]).toBe('/api/chat/resources?type=image&limit=120'));
    await screen.findByRole('heading', { name: '2025年10月' });
    fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
    expect(await screen.findByRole('heading', { name: '2025年9月' })).toBeTruthy();
    expect(calls[2]).toBe('/api/chat/resources?type=image&limit=120&before=k1');
    expect(screen.queryByRole('button', { name: '加载更多' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /全部会话/ }));
    const list = await screen.findByRole('listbox', { name: '会话' });
    fireEvent.click(within(list).getByRole('option', { name: '家人群' }));
    await waitFor(() => expect(calls.at(-1)).toBe('/api/chat/resources?session=room%40chatroom&type=image&limit=120'));
    fireEvent.click(await screen.findByRole('button', { name: '清除筛选' }));
    await waitFor(() => expect(calls.at(-1)).toBe('/api/chat/resources?type=image&limit=120'));
  });

  it('says why a thumbnail cannot be shown', async () => {
    serve({ '': { items: [item(1, 'image', OCT)], hasMore: false, nextCursor: null } });
    const { container } = page();
    const img = await waitFor(() => { const found = container.querySelector('.resource-thumb img'); if (!found) throw new Error('no thumbnail'); return found; });
    fireEvent.error(img);
    expect(await screen.findByText('WXGF 图片无法转换：暂不支持它的格式（P and B slices (inter prediction)）')).toBeTruthy();
    expect(screen.getByText('图片无法显示')).toBeTruthy();
  });

  it('reads the next page within 800px of the bottom of the scroll area, or of the window where the area does not scroll', async () => {
    const watched: { root: Element | Document | null | undefined; rootMargin?: string; fire(): void }[] = [];
    vi.stubGlobal('IntersectionObserver', class {
      constructor(private readonly callback: IntersectionObserverCallback, private readonly options: IntersectionObserverInit = {}) {}
      observe(target: Element) {
        watched.push({ root: this.options.root, rootMargin: this.options.rootMargin, fire: () => this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this as unknown as IntersectionObserver) });
      }
      disconnect() {}
    });
    // the area holds more than it shows: it scrolls itself
    const tall = (area: HTMLElement, scrolls: boolean) => {
      Object.defineProperty(area, 'scrollHeight', { configurable: true, value: scrolls ? 3000 : 600 });
      Object.defineProperty(area, 'clientHeight', { configurable: true, value: 600 });
    };
    const area = document.createElement('section');
    tall(area, true);
    const calls = serve({
      '': { items: [item(1, 'image', OCT)], hasMore: true, nextCursor: 'k1' },
      k1: { items: [item(2, 'image', SEP)], hasMore: true, nextCursor: 'k2' },
    });
    render(<ResourcesPage filter={ALL_RESOURCES} scroller={{ current: area }} onPreview={() => {}} onOpen={() => {}} />);
    await waitFor(() => expect(watched).toHaveLength(1));
    expect(watched[0]).toMatchObject({ root: area, rootMargin: '0px 0px 800px 0px' });
    await act(async () => watched[0].fire());
    await waitFor(() => expect(watched).toHaveLength(2));
    expect(calls.at(-1)).toBe('/api/chat/resources?type=image&type=video&type=voice&type=file&limit=120&before=k1');

    // a phone: the area grows with the grid, and the document scrolls
    tall(area, false);
    cleanup();
    watched.length = 0;
    render(<ResourcesPage filter={ALL_RESOURCES} scroller={{ current: area }} onPreview={() => {}} onOpen={() => {}} />);
    await waitFor(() => expect(watched).toHaveLength(1));
    expect(watched[0].root).toBeNull();
  });

  it('tells an empty listing from an empty filter', async () => {
    serve({});
    page();
    expect(await screen.findByText('没有找到图片、视频、语音或文件。')).toBeTruthy();
    pick('语音');
    expect(await screen.findByText('没有找到语音。')).toBeTruthy();
  });
});
