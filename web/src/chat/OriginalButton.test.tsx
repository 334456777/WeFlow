import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ParsedMessage } from '../api/messages';
import { imagePreview } from './MessageView';
import { OriginalButton } from './OriginalButton';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (data: unknown) => new Response(JSON.stringify({ success: true, data }));
const message = { _cursor: 'c3', localId: 3, localType: 3, createTime: 1_700_000_000, isSend: 0, parsedContent: '' } as ParsedMessage;

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
    render(<OriginalButton hd={imagePreview('wxid_a', message, '图片').hd!} onToggle={onShow} />);
    const button = await screen.findByRole('button', { name: /^查看高清原图 2\.3 MB$/ });
    expect(fetch.mock.calls[0][0]).toContain('/api/chat/image-info?session-id=wxid_a&msg-id=3&create-time=1700000000');
    fireEvent.click(button);
    expect(onShow).toHaveBeenCalledWith(true);
  });

  it('cannot be pressed until WeChat has downloaded it, and says why', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: false, hdBytes: 2_400_000 })));
    const onShow = vi.fn();
    render(<OriginalButton hd={imagePreview('wxid_a', message, '图片').hd!} onToggle={onShow} />);
    const button = await screen.findByRole('button', { name: /还没有下载到这台电脑/ });
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(button.title).toBe('高清原图（2.3 MB）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试');
    fireEvent.click(button);
    expect(onShow).not.toHaveBeenCalled();
  });

  it('says so when the picture has no original, and while it shows the original', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: false })));
    const hd = imagePreview('wxid_a', message, '图片').hd!;
    render(<OriginalButton hd={hd} onToggle={vi.fn()} />);
    expect((await screen.findByRole('button', { name: /没有高清原图/ })).title).toBe('这张图片没有高清原图');
  });

  it('goes back to the image opened when pressed again (#338)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: true, hd: true, hdBytes: 2_400_000 })));
    const onToggle = vi.fn();
    const hd = imagePreview('wxid_a', message, '图片').hd!;
    render(<OriginalButton hd={{ ...hd, shown: true }} onToggle={onToggle} />);
    const button = await screen.findByRole('button', { name: /^正在显示高清原图，点此换回点开时的图片$/ });
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.textContent).toBe('原图 2.3 MB');
    fireEvent.click(button);
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it('spins while the original is on its way', () => {
    const hd = imagePreview('wxid_a', message, '图片').hd!;
    const { rerender } = render(<OriginalButton hd={{ ...hd, shown: true, loading: true }} onToggle={vi.fn()} />);
    expect(screen.getByTitle(/^正在载入高清原图/).querySelector('.spin')).not.toBeNull();
    rerender(<OriginalButton hd={{ ...hd, shown: true, loading: false }} onToggle={vi.fn()} />);
    expect(screen.getByTitle(/^正在显示高清原图/).querySelector('.spin')).toBeNull();
  });

  it('is already shown when only the original is on this computer', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json({ thumb: true, normal: false, hd: true, hdBytes: 2_400_000 })));
    render(<OriginalButton hd={imagePreview('wxid_a', message, '图片').hd!} onToggle={vi.fn()} />);
    expect(await screen.findByTitle('正在显示高清原图')).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });
});
