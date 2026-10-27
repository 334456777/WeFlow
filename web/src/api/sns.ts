// Moments (#202): the timeline (`sns timeline`), who posts and how often (`sns post-counts`), the totals
// (`sns stats`), all read in the server's process, and the images and videos of a post (`/api/sns/media`).
import { apiUrl, get, token, type Params } from './client';
import type * as Contract from './contract.gen';

/** An image or video of a post, as the timeline gives it: addresses on WeChat's servers and the key to them. */
export interface SnsMedia {
  url: string;
  thumb?: string;
  key?: string;
  md5?: string;
  livePhoto?: { url: string; thumb?: string; key?: string };
}

export interface SnsComment {
  id?: string;
  username?: string;
  nickname: string;
  content: string;
  /** The comment answered, and who wrote it. */
  refCommentId?: string;
  refNickname?: string;
}

export interface SnsLocation {
  poiName?: string;
  poiAddress?: string;
  city?: string;
  country?: string;
}

export interface SnsPost {
  id: string;
  username: string;
  nickname: string;
  avatarUrl?: string;
  /** Unix seconds. */
  createTime: number;
  /** WeChat's kind of post: 1 pictures, 2 text, 15 a video, 7 media that need no key, … */
  type: number;
  contentDesc?: string;
  media?: SnsMedia[];
  likes?: string[];
  comments?: SnsComment[];
  location?: SnsLocation | null;
}

/** What the timeline is narrowed to. */
export interface SnsFilter {
  /** One poster, or everyone. */
  username: string | null;
  keyword: string;
  /** Unix seconds, 0 for no limit. */
  start: number;
  end: number;
}

export const NO_FILTER: SnsFilter = { username: null, keyword: '', start: 0, end: 0 };

/** Posts per page. */
export const SNS_PAGE = 20;

export async function getTimeline(filter: SnsFilter, offset: number, signal?: AbortSignal): Promise<SnsPost[]> {
  const params: Params = { limit: SNS_PAGE, offset };
  if (filter.username) params.user = filter.username;
  if (filter.keyword.trim()) params.keyword = filter.keyword.trim();
  if (filter.start) params.start = filter.start;
  if (filter.end) params.end = filter.end;
  const data = await get<{ timeline?: SnsPost[] }>('api/sns/timeline', params, signal);
  return data?.timeline ?? [];
}

/** Posts per poster, from the counts WeChat keeps (refreshed every few minutes). */
export async function getPostCounts(signal?: AbortSignal): Promise<Record<string, number>> {
  return (await get<Record<string, number> | null>('api/sns/post-counts', { 'prefer-cache': true }, signal)) ?? {};
}

/** The contract's (#430). */
export type SnsStats = Contract.SnsStats;

export async function getSnsStats(signal?: AbortSignal): Promise<SnsStats> {
  const data = await get<SnsStats | null>('api/sns/stats', { fast: true }, signal);
  return { totalPosts: data?.totalPosts ?? 0, totalFriends: data?.totalFriends ?? 0, myPosts: data?.myPosts ?? null };
}

/** The servers `/api/sns/media` fetches from (as `is_wechat_media_url` on the server); other addresses are links. */
const SERVERS = ['.qpic.cn', '.wechat.com', '.video.qq.com', '.tc.qq.com'];

export function isWechatMedia(url: string | undefined): url is string {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' && !parsed.port && SERVERS.some(s => parsed.hostname.endsWith(s));
  } catch {
    return false;
  }
}

/** A video by its address, as WeChat's apps tell one. */
export function isVideo(media: SnsMedia): boolean {
  const url = media.url.toLowerCase();
  return !url.includes('vweixinthumb') && (url.includes('snsvideodownload') || url.includes('.mp4') || url.includes('video'));
}

/**
 * Where a page gets an image or video of a post: through the server, which fetches it once from WeChat's servers,
 * decrypts it and keeps it. Posts of type 7 hold media that need no key.
 */
export function snsMediaUrl(url: string, key: string | undefined, postType: number): string {
  const bearer = token();
  return apiUrl('api/sns/media', {
    url,
    ...(key && postType !== 7 ? { key } : {}),
    ...(bearer ? { access_token: bearer } : {}),
  });
}
