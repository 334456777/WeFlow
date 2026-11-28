// 朋友圈（#202）：时间线（`sns timeline`）、谁发了多少条（`sns post-counts`）和总数（`sns stats`），
// 都在服务进程内读取；以及动态里的图片和视频（`/api/sns/media`）。
import { get, mediaAddress, token, type Params } from './client';
import type * as Contract from './contract.gen';

/** 一条动态：其中的图片和视频（微信服务器上的地址和解密用的密钥）、评论和发布的位置，都是契约的类型（#430）。 */
export type SnsPost = Contract.SnsPost;
/** 动态里的一张图片或一段视频。 */
export type SnsMedia = Contract.SnsMedia;
/** 动态下的一条评论。 */
export type SnsComment = Contract.SnsComment;
/** 动态发布的位置。 */
export type SnsLocation = Contract.SnsLocation;

/** 时间线的筛选条件。 */
export interface SnsFilter {
  /** 只看这个人的动态；为 null 时看所有人的。 */
  username: string | null;
  keyword: string;
  /** Unix 秒数，0 表示不限。 */
  start: number;
  end: number;
}

/** 不加筛选的整条时间线。 */
export const NO_FILTER: SnsFilter = { username: null, keyword: '', start: 0, end: 0 };

/** 每页的动态数。 */
export const SNS_PAGE = 20;

/** 按 `filter` 筛选的 `sns timeline` 从第 `offset` 条起的一页，在服务进程内读取。 */
export async function getTimeline(filter: SnsFilter, offset: number, signal?: AbortSignal): Promise<SnsPost[]> {
  const params: Params = { limit: SNS_PAGE, offset };
  if (filter.username) params.user = filter.username;
  if (filter.keyword.trim()) params.keyword = filter.keyword.trim();
  if (filter.start) params.start = filter.start;
  if (filter.end) params.end = filter.end;
  const data = await get<Contract.SnsTimeline | null>('api/sns/timeline', params, signal);
  return data?.timeline ?? [];
}

/** `sns post-counts`：每个人发的动态数；服务端保存的上一次的计数没有过期时直接用它（`prefer-cache`）。 */
export async function getPostCounts(signal?: AbortSignal): Promise<Record<string, number>> {
  return (await get<Record<string, number> | null>('api/sns/post-counts', { 'prefer-cache': true }, signal)) ?? {};
}

/** `sns stats` 的回答：契约的类型（#430）。 */
export type SnsStats = Contract.SnsStats;

/** `sns stats`：朋友圈的统计，服务端保存的计数没有过期时直接用它；没有给出的总数当作 0，没有 `myPosts` 时为 null。 */
export async function getSnsStats(signal?: AbortSignal): Promise<SnsStats> {
  const data = await get<SnsStats | null>('api/sns/stats', { 'prefer-cache': true }, signal);
  return { totalPosts: data?.totalPosts ?? 0, totalFriends: data?.totalFriends ?? 0, myPosts: data?.myPosts ?? null };
}

/** `/api/sns/media` 会去下载的服务器（与服务端的 `is_wechat_media_url` 一致）；其他地址只作为链接。 */
const SERVERS = ['.qpic.cn', '.wechat.com', '.video.qq.com', '.tc.qq.com'];

/** `url` 是否是微信媒体服务器的 https 地址，即 `/api/sns/media` 会去下载的地址。 */
export function isWechatMedia(url: string | undefined): url is string {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' && !parsed.port && SERVERS.some(s => parsed.hostname.endsWith(s));
  } catch {
    return false;
  }
}

/** 按地址判断是否是视频，与微信客户端的判断方式一致。 */
export function isVideo(media: SnsMedia): boolean {
  const url = media.url.toLowerCase();
  return !url.includes('vweixinthumb') && (url.includes('snsvideodownload') || url.includes('.mp4') || url.includes('video'));
}

/**
 * 网页获取动态里的图片或视频的地址：经过服务端，服务端从微信的服务器下载一次、解密后保存下来。
 * 类型为 7 的动态里的媒体不需要密钥。
 */
export function snsMediaUrl(url: string, key: string | undefined, postType: number): string {
  const bearer = token();
  return mediaAddress('api/sns/media', {
    url,
    ...(key && postType !== 7 ? { key } : {}),
    ...(bearer ? { access_token: bearer } : {}),
  });
}
