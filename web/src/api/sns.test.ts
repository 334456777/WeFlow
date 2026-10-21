import { afterEach, describe, expect, it, vi } from '#test';
import { setToken } from './client';
import { getTimeline, isVideo, isWechatMedia, NO_FILTER, snsMediaUrl } from './sns';

afterEach(() => { vi.unstubAllGlobals(); setToken(null); });

describe('Moments media (#202)', () => {
  it('tells WeChat media servers from shared links, as the server does', () => {
    expect(isWechatMedia('https://shmmsns.qpic.cn/mmsns/x/0')).toBe(true);
    expect(isWechatMedia('https://szzjwxsns.video.qq.com/102/snsvideodownload?m=1')).toBe(true);
    expect(isWechatMedia('https://y.music.163.com/m/song?id=1')).toBe(false);
    expect(isWechatMedia('http://shmmsns.qpic.cn/mmsns/x/0')).toBe(false);
    expect(isWechatMedia('https://shmmsns.qpic.cn:8443/x')).toBe(false);
    expect(isWechatMedia(undefined)).toBe(false);
  });

  it('knows a video by its address, not by the cover of one', () => {
    expect(isVideo({ url: 'https://a.video.qq.com/102/snsvideodownload?m=1' })).toBe(true);
    expect(isVideo({ url: 'https://vweixinthumb.tc.qq.com/150/x' })).toBe(false);
    expect(isVideo({ url: 'https://a.qpic.cn/mmsns/x/0' })).toBe(false);
  });

  it('asks the server for a medium with its key and the tab token; posts of type 7 need no key', () => {
    setToken('tok');
    const url = 'https://a.qpic.cn/mmsns/x/0?token=T&idx=1';
    expect(snsMediaUrl(url, '123', 1)).toBe('/api/sns/media?url=' + encodeURIComponent(url).replace(/%20/g, '+') + '&key=123&access_token=tok');
    expect(snsMediaUrl(url, '123', 7)).not.toContain('key=');
  });

  it('reads the timeline narrowed to a poster, words and days, a page at a time', async () => {
    const fetch = vi.fn(async () => new Response(JSON.stringify({ success: true, data: { timeline: [] } })));
    vi.stubGlobal('fetch', fetch);
    await getTimeline({ username: 'wxid_a', keyword: ' 海边 ', start: 100, end: 200 }, 20);
    const asked = new URL((fetch.mock.calls[0] as unknown as [string])[0], 'http://x');
    expect(asked.pathname).toBe('/api/sns/timeline');
    expect(Object.fromEntries(asked.searchParams)).toEqual({ limit: '20', offset: '20', user: 'wxid_a', keyword: '海边', start: '100', end: '200' });
    await getTimeline(NO_FILTER, 0);
    expect((fetch.mock.calls[1] as unknown as [string])[0]).toBe('/api/sns/timeline?limit=20&offset=0');
  });
});
