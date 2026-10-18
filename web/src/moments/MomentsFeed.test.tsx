import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { NO_FILTER, SNS_PAGE, type SnsPost } from '../api/sns';
import { MomentsFeed } from './MomentsFeed';
import { peopleOf } from './useMomentPeople';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const T = 1760000000;
const post = (id: number, more: Partial<SnsPost> = {}): SnsPost => ({ id: String(id), username: 'wxid_a', nickname: '林沐', createTime: T - id * 3600, type: 2, contentDesc: '第 ' + id + ' 条', ...more });
const page = (posts: SnsPost[]) => new Response(JSON.stringify({ success: true, data: { timeline: posts } }));
const scroller = { current: null };

async function feed(...replies: SnsPost[][]) {
  const fetch = vi.fn();
  for (const posts of replies) fetch.mockResolvedValueOnce(page(posts));
  vi.stubGlobal('fetch', fetch);
  const onPreview = vi.fn();
  render(<MomentsFeed filter={NO_FILTER} scroller={scroller} onPreview={onPreview} />);
  await act(async () => {});
  return { fetch, onPreview };
}

describe('the Moments timeline (#202)', () => {
  it('shows a post with its pictures, video, place, likes and comments', async () => {
    const image = 'https://a.qpic.cn/mmsns/p/0';
    const video = 'https://a.video.qq.com/102/snsvideodownload?m=1';
    const { onPreview } = await feed([post(1, {
      type: 1, contentDesc: '海边\n日落',
      media: [{ url: image, thumb: image, key: '11' }, { url: video, thumb: 'https://a.video.qq.com/cover', key: '22' }],
      location: { poiName: '外滩', city: '上海' },
      likes: ['小周', '阿杰'],
      comments: [{ nickname: '小周', content: '好看' }, { nickname: '林沐', content: '谢谢', refNickname: '小周' }],
    })]);
    const card = screen.getByRole('article');
    expect(card.textContent).toContain('林沐');
    expect(card.querySelector('.moment-text')!.textContent).toBe('海边\n日落');
    const pictures = [...card.querySelectorAll('.moment-tile img')].map(img => img.getAttribute('src')!);
    expect(pictures[0]).toContain('/api/sns/media?url=' + encodeURIComponent(image));
    expect(pictures[0]).toContain('key=11');
    // a video shows its cover with a play mark, and plays through the server
    expect(card.querySelectorAll('.moment-play')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: '预览林沐的视频 2' }));
    expect(onPreview).toHaveBeenCalledWith(expect.objectContaining({ type: 'video', src: expect.stringContaining(encodeURIComponent(video)) }));
    expect(card.querySelector('.moment-place')!.textContent).toBe('外滩');
    expect(card.querySelector('.moment-likes')!.textContent).toBe('小周，阿杰');
    expect([...card.querySelectorAll('.moment-comments li')].map(li => li.textContent)).toEqual(['小周：好看', '林沐 回复 小周：谢谢']);
  });

  it('shows a shared link as a link, not as a picture the server would refuse', async () => {
    await feed([post(1, { type: 1, media: [{ url: 'https://y.music.163.com/m/song?id=1', thumb: 'https://a.qpic.cn/mmsns/c/0', key: '5' }] })]);
    const link = screen.getByRole('link');
    expect(link.getAttribute('href')).toBe('https://y.music.163.com/m/song?id=1');
    expect(link.textContent).toContain('y.music.163.com');
    expect(document.querySelectorAll('.moment-tile')).toHaveLength(0);
  });

  it('reads the next page from where it ends, without showing a post twice', async () => {
    const first = Array.from({ length: SNS_PAGE }, (_, i) => post(i));
    const { fetch } = await feed(first, [post(SNS_PAGE - 1), post(SNS_PAGE), post(SNS_PAGE + 1)]);
    expect(screen.getAllByRole('article')).toHaveLength(SNS_PAGE);
    fireEvent.click(screen.getByRole('button', { name: '加载更早的朋友圈' }));
    await act(async () => {});
    expect(fetch.mock.calls[1][0]).toContain('offset=' + SNS_PAGE);
    expect(screen.getAllByRole('article')).toHaveLength(SNS_PAGE + 2);
    expect(screen.getByText('没有更早的朋友圈了')).toBeTruthy();
  });

  it('says when nothing matches', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(page([])));
    render(<MomentsFeed filter={{ ...NO_FILTER, keyword: '不存在' }} scroller={scroller} onPreview={vi.fn()} />);
    await act(async () => {});
    expect(screen.getByText('没有符合筛选条件的朋友圈。')).toBeTruthy();
  });
});

describe('who posts', () => {
  it('names posters from the contacts, this account as 我, most posts first', () => {
    const contacts = [
      { username: 'wxid_a', remark: '阿杰', nick_name: 'AJ', small_head_url: 'https://wx.qlogo.cn/a' },
      { username: 'wxid_b', nick_name: '小周' },
    ];
    expect(peopleOf({ wxid_a: 3, wxid_b: 9, wxid_me: 4, wxid_gone: 1, wxid_none: 0 }, contacts, 'wxid_me')).toEqual([
      { username: 'wxid_b', name: '小周', avatarUrl: undefined, posts: 9 },
      { username: 'wxid_me', name: '我', avatarUrl: undefined, posts: 4 },
      { username: 'wxid_a', name: '阿杰', avatarUrl: 'https://wx.qlogo.cn/a', posts: 3 },
      { username: 'wxid_gone', name: 'wxid_gone', avatarUrl: undefined, posts: 1 },
    ]);
  });
});
