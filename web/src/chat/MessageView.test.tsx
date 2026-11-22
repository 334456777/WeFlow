import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { ParsedMessage } from '../api/messages';
import { MessageView } from './MessageView';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const video = (localId: number) => ({ _cursor: 'c' + localId, localId, localType: 43, createTime: 1_700_000_000, isSend: 0, parsedContent: '' }) as ParsedMessage;
const failed = (message: string) => new Response(JSON.stringify({ success: false, error: { code: 'runtime_error', message, details: { failureKind: 'not_found' } } }), { status: 404 });

/** The video message `localId`, its thumbnail failed: the first video is not on this computer, the second plays without a thumbnail. */
function failedThumbnail(localId: number) {
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.searchParams.get('local-id') === '1') return failed('该视频不在本机：请先在微信中打开该视频');
    return url.searchParams.has('thumb') ? failed('该视频没有缩略图或封面') : new Response(new Uint8Array([0]), { status: 206 });
  }));
  const { container } = render(<MessageView sessionId="wxid_a" message={video(localId)} onPreview={vi.fn()} />);
  const button = screen.getByRole('button', { name: '播放视频' });
  // laid over the thumbnail: a child of the button
  const badgeOver = () => [...button.children].some(child => child.classList.contains('play-badge'));
  expect(badgeOver()).toBe(true);
  fireEvent.error(container.querySelector('img')!);
  return { button, badgeOver };
}

describe('a video message whose thumbnail fails (#1073)', () => {
  it('draws no play button when the video is not on this computer', async () => {
    const { button } = failedThumbnail(1);
    expect(await screen.findByText('视频无法显示')).toBeTruthy();
    // the answer about the video has come, and still no button
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    await act(() => new Promise(resolve => setTimeout(resolve, 0)));
    expect(button.querySelector('.play-badge')).toBeNull();
  });

  it('draws the play button in place of the warning when the video plays', async () => {
    const { button, badgeOver } = failedThumbnail(2);
    expect(await screen.findByText('缩略图无法显示')).toBeTruthy();
    expect(button.querySelector('.media-missing')?.firstElementChild?.className).toBe('play-badge');
    expect(badgeOver()).toBe(false);
  });
});
