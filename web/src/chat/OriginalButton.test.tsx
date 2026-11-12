import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { ParsedMessage } from '../api/messages';
import { imagePreview } from './MessageView';
import { OriginalButton } from './OriginalButton';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }));
const failure = (code: string) => new Response(JSON.stringify({ success: false, error: { code, message: 'no hook' } }), { status: 501 });
const message = { _cursor: 'c3', localId: 3, localType: 3, createTime: 1_700_000_000, isSend: 0, parsedContent: '' } as ParsedMessage;
const hdOf = () => imagePreview('wxid_a', message, '图片').hd!;
const ASKED = '/api/image/download-original';

describe('the original of a picture (#194)', () => {
  it('opens the image WeChat shows when it is opened, with its original beside', () => {
    const preview = imagePreview('wxid_a', message, '图片');
    expect(preview.src).toContain('/api/chat/image-data?session-id=wxid_a&msg-id=3&create-time=1700000000&variant=normal');
    expect(preview.hd).toMatchObject({ sessionId: 'wxid_a', localId: 3, createTime: 1_700_000_000 });
    expect(preview.hd?.src).toContain('variant=hd');
  });

  it('shows the original once it is on this computer', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: true, hdBytes: 2_400_000 }));
    vi.stubGlobal('fetch', fetch);
    const onShow = vi.fn();
    render(<OriginalButton hd={hdOf()} canDownload onToggle={onShow} />);
    const button = await screen.findByRole('button', { name: /^查看高清原图 2\.3 MB$/ });
    expect(fetch.mock.calls[0][0]).toContain('/api/chat/image-info?session-id=wxid_a&msg-id=3&create-time=1700000000');
    fireEvent.click(button);
    expect(onShow).toHaveBeenCalledWith(true);
  });

  it('cannot be pressed until WeChat has downloaded it, and says why', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: false, hdBytes: 2_400_000 })));
    const onShow = vi.fn();
    render(<OriginalButton hd={hdOf()} canDownload={false} onToggle={onShow} />);
    const button = await screen.findByRole('button', { name: /还没有下载到这台电脑/ });
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(button.title).toBe('高清原图（2.3 MB）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试');
    fireEvent.click(button);
    expect(onShow).not.toHaveBeenCalled();
  });

  it('says so when the picture has no original, and while it shows the original', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: false })));
    render(<OriginalButton hd={hdOf()} canDownload onToggle={vi.fn()} />);
    expect((await screen.findByRole('button', { name: /没有高清原图/ })).title).toBe('这张图片没有高清原图');
  });

  it('goes back to the image opened when pressed again (#338)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: true, hdBytes: 2_400_000 })));
    const onToggle = vi.fn();
    render(<OriginalButton hd={{ ...hdOf(), shown: true }} canDownload onToggle={onToggle} />);
    const button = await screen.findByRole('button', { name: /^正在显示高清原图，点此换回点开时的图片$/ });
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.textContent).toBe('原图 2.3 MB');
    fireEvent.click(button);
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it('spins while the original is on its way', () => {
    // what is on this computer does not matter here: the question is never answered
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})));
    const hd = hdOf();
    const { rerender } = render(<OriginalButton hd={{ ...hd, shown: true, loading: true }} canDownload onToggle={vi.fn()} />);
    expect(screen.getByTitle(/^正在载入高清原图/).querySelector('.spin')).not.toBeNull();
    rerender(<OriginalButton hd={{ ...hd, shown: true, loading: false }} canDownload onToggle={vi.fn()} />);
    expect(screen.getByTitle(/^正在显示高清原图/).querySelector('.spin')).toBeNull();
  });

  it('is already shown when only the original is on this computer', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: false, hd: true, hdBytes: 2_400_000 })));
    render(<OriginalButton hd={hdOf()} canDownload onToggle={vi.fn()} />);
    expect(await screen.findByTitle('正在显示高清原图')).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });
});

describe('asking WeChat for the original (#766)', () => {
  /** A server that takes the request unless `refuse`, and says the original is here once `downloaded` is set. */
  function server({ refuse = false } = {}) {
    const state = { asked: false, downloaded: false };
    const fetch = vi.fn((url: string, _init?: RequestInit) => {
      if (String(url).includes(ASKED)) {
        state.asked = true;
        return Promise.resolve(refuse ? failure('unsupported') : json({ success: true, path: 'C:\\h.dat', bytes: 2_400_000 }));
      }
      return Promise.resolve(json({ thumb: true, normal: true, hd: state.downloaded, hdBytes: 2_400_000, normalBytes: 240_000 }));
    });
    return { state, fetch };
  }

  it('asks for it when it is missing, waits, and shows it once it has come', async () => {
    vi.useFakeTimers();
    const { state, fetch } = server();
    vi.stubGlobal('fetch', fetch);
    const onShow = vi.fn();
    render(<OriginalButton hd={hdOf()} canDownload onToggle={onShow} />);
    await act(async () => {});
    // the picture is here, its original is not: the server is asked, and the button waits for the file
    const waiting = screen.getByRole('button', { name: /正在下载原图/ });
    expect(waiting.getAttribute('aria-disabled')).toBe('true');
    expect(waiting.querySelector('.spin')).not.toBeNull();
    expect(fetch.mock.calls[1][0]).toBe(ASKED);
    expect(fetch.mock.calls[1][1]).toMatchObject({ method: 'POST' });
    // WeChat downloads it at its next download, which the button looks for once in a while
    state.downloaded = true;
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(onShow).toHaveBeenCalledWith(true);
    expect(fetch.mock.calls.filter(call => String(call[0]).includes('/api/chat/image-info'))).toHaveLength(2);
  });

  it('says when the server cannot ask WeChat at all', async () => {
    const { fetch } = server({ refuse: true });
    vi.stubGlobal('fetch', fetch);
    render(<OriginalButton hd={hdOf()} canDownload onToggle={vi.fn()} />);
    const button = await screen.findByRole('button', { name: /没能让微信下载原图/ });
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(fetch.mock.calls.filter(call => String(call[0]).includes(ASKED))).toHaveLength(1);
  });

  it('gives up after a minute and says WeChat has not downloaded it', async () => {
    vi.useFakeTimers();
    const { fetch } = server();
    vi.stubGlobal('fetch', fetch);
    render(<OriginalButton hd={hdOf()} canDownload onToggle={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByRole('button', { name: /正在下载原图/ })).toBeTruthy();
    for (let i = 0; i < 30; i++) await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    const button = screen.getByRole('button', { name: /微信还没有下载原图/ });
    expect(button.title).toContain('在微信里打开这张图片');
    expect(button.querySelector('.spin')).toBeNull();
  });
});
