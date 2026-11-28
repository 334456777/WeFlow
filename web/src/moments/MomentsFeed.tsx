// 朋友圈时间线（#202）：最新的在前，一次读一页，每条带文字、图片、视频、分享的链接、位置、点赞和评论。
// 图片和视频经过 `/api/sns/media` 读取，它从微信的服务器下载一次后保存下来。
import { useEffect, useRef, useState } from 'react';
import { ArrowClockwise, CircleNotch, Heart, ImageBroken, LinkSimple, MapPin, Play } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getTimeline, isVideo, isWechatMedia, snsMediaUrl, SNS_PAGE, type SnsFilter, type SnsMedia, type SnsPost } from '../api/sns';
import { Avatar } from '../Avatar';
import type { Preview } from '../MessageMedia';
import { dayOf, formatClock, formatDayHeading } from '../format';
import { MomentsSkeleton } from '../Skeleton';
import { t } from '../i18n';
import { safeUrl } from '../links';

type FeedState =
  | { status: 'loading' }
  | { status: 'ready'; posts: SnsPost[]; hasMore: boolean; more: boolean; error?: string }
  | { status: 'error'; message: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取朋友圈失败');

/** 朋友圈时间线。每个服务端或账号、每个筛选条件要给它不同的 `key`，换了之后从头开始。 */
export function MomentsFeed({ filter, scroller, onPreview }: {
  filter: SnsFilter;
  /** 滚动的面板，用来在滚到末尾之前读取下一页。 */
  scroller: React.RefObject<HTMLElement | null>;
  onPreview(preview: Preview): void;
}) {
  const [state, setState] = useState<FeedState>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const request = useRef<AbortController | null>(null);
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    request.current = controller;
    setState({ status: 'loading' });
    getTimeline(filter, 0, controller.signal).then(
      posts => { if (!controller.signal.aborted) setState({ status: 'ready', posts, hasMore: posts.length === SNS_PAGE, more: false }); },
      problem => { if (!controller.signal.aborted) setState({ status: 'error', message: errorOf(problem) }); },
    );
    return () => controller.abort();
  }, [filter, round]);

  const more = () => {
    const signal = request.current?.signal;
    if (signal?.aborted || state.status !== 'ready' || state.more || !state.hasMore) return;
    const shown = state.posts;
    setState({ ...state, more: true, error: undefined });
    getTimeline(filter, shown.length, signal).then(
      posts => {
        if (signal?.aborted) return;
        const seen = new Set(shown.map(p => p.id));
        setState(old => (old.status === 'ready' && old.posts === shown
          ? { ...old, posts: [...shown, ...posts.filter(p => !seen.has(p.id))], hasMore: posts.length === SNS_PAGE, more: false }
          : old));
      },
      problem => { if (!signal?.aborted) setState(old => (old.status === 'ready' && old.posts === shown ? { ...old, more: false, error: errorOf(problem) } : old)); },
    );
  };

  // 列表的末尾接近可见区域（800px 以内）时读取下一页
  const latestMore = useRef(more);
  latestMore.current = more;
  useEffect(() => {
    const target = end.current;
    if (!target || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) latestMore.current(); },
      { root: scroller.current, rootMargin: '0px 0px 800px 0px' });
    observer.observe(target);
    return () => observer.disconnect();
  }, [scroller, state.status]);

  if (state.status === 'loading') return <MomentsSkeleton label={t('正在读取朋友圈')} />;
  if (state.status === 'error') {
    return <div className="pane-message" role="alert"><p>{state.message}</p>
      <button type="button" className="secondary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  }
  if (!state.posts.length) {
    const narrowed = filter.keyword.trim() || filter.start || filter.end;
    return <div className="pane-message"><p>{narrowed ? t('没有符合筛选条件的朋友圈。') : t('本机没有这些朋友圈。')}</p></div>;
  }
  return <div className="moments" aria-label={t('朋友圈')}>
    {state.posts.map(post => <MomentPost key={post.id} post={post} onPreview={onPreview} />)}
    <div ref={end} className="moments-end">
      {state.error && <p role="alert">{state.error}</p>}
      {state.hasMore
        ? <button type="button" className="secondary-button" onClick={more} disabled={state.more}>{state.more ? <><CircleNotch size={14} className="spin" />{t('正在读取')}</> : t('加载更早的朋友圈')}</button>
        : <p>{t('没有更早的朋友圈了')}</p>}
    </div>
  </div>;
}

function when(seconds: number): string {
  return formatDayHeading(dayOf(seconds)) + ' ' + formatClock(seconds);
}

/** 一条朋友圈的媒体：微信服务器上的图片和视频，以及分享的链接（其他地址）。 */
function partsOf(post: SnsPost): { pictures: SnsMedia[]; link?: SnsMedia } {
  const media = post.media ?? [];
  return { pictures: media.filter(m => isWechatMedia(m.url)), link: media.find(m => m.url && !isWechatMedia(m.url)) };
}

/** 一条朋友圈：发布者、文字、分享的链接、图片或视频、发布的位置、点赞和评论。 */
export function MomentPost({ post, onPreview }: { post: SnsPost; onPreview(preview: Preview): void }) {
  const { pictures: shown, link } = partsOf(post);
  const place = post.location?.poiName || post.location?.city;
  const name = post.nickname || post.username;
  return <article className="moment" aria-label={name + t('，') + when(post.createTime)}>
    <Avatar name={name} url={post.avatarUrl} />
    <div className="moment-body">
      <header><strong className="clamp-1">{name}</strong><time dateTime={new Date(post.createTime * 1000).toISOString()}>{when(post.createTime)}</time></header>
      {post.contentDesc?.trim() && <p className="moment-text">{post.contentDesc.trim()}</p>}
      {link && <SharedLink link={link} post={post} />}
      {shown.length > 0 && <div className={'moment-media count-' + Math.min(shown.length, 9)}>
        {shown.slice(0, 9).map((m, i) => <MediaTile key={i} media={m} post={post} index={i} onPreview={onPreview} />)}
      </div>}
      {place && <p className="moment-place"><MapPin size={12} aria-hidden="true" />{place}</p>}
      {(post.likes?.length || post.comments?.length) ? <div className="moment-social">
        {post.likes?.length ? <p className="moment-likes"><Heart size={12} weight="fill" aria-label={t('赞')} />{post.likes.join(t('，'))}</p> : null}
        {post.comments?.length ? <ul className="moment-comments" aria-label={t('评论')}>
          {post.comments.map((c, i) => <li key={c.id || i}><b>{c.nickname}</b>{c.refNickname && <> {t('回复')} <b>{c.refNickname}</b></>}{t('：')}{c.content}</li>)}
        </ul> : null}
      </div> : null}
    </div>
  </article>;
}

const hostOf = (url: string) => { try { return new URL(url).hostname || url; } catch { return url; } };

/** 朋友圈分享的网页：地址是网页地址时是指向它的链接，否则是不带链接的同样的卡片。 */
function SharedLink({ link, post }: { link: SnsMedia; post: SnsPost }) {
  const url = safeUrl(link.url);
  const inner = <>
    {isWechatMedia(link.thumb) ? <MediaImage src={snsMediaUrl(link.thumb, link.key, post.type)} label={t('链接封面')} /> : <span className="moment-link-icon" aria-hidden="true"><LinkSimple size={20} /></span>}
    <span className="clamp-2">{hostOf(link.url)}</span>
  </>;
  return url
    ? <a className="moment-link" href={url} target="_blank" rel="noreferrer noopener">{inner}</a>
    : <div className="moment-link">{inner}</div>;
}

function MediaTile({ media, post, index, onPreview }: { media: SnsMedia; post: SnsPost; index: number; onPreview(preview: Preview): void }) {
  const video = isVideo(media);
  const who = post.nickname || post.username;
  const title = video ? t('{name}的视频 {n}', { name: who, n: index + 1 }) : t('{name}的图片 {n}', { name: who, n: index + 1 });
  const cover = isWechatMedia(media.thumb) ? media.thumb : !video && isWechatMedia(media.url) ? media.url : undefined;
  const open = () => {
    if (video) onPreview({ type: 'video', title, src: snsMediaUrl(media.url, media.key, post.type), poster: cover && snsMediaUrl(cover, media.key, post.type) });
    else onPreview({ type: 'image', title, src: snsMediaUrl(isWechatMedia(media.url) ? media.url : cover!, media.key, post.type) });
  };
  return <button type="button" className="moment-tile" aria-label={t('预览{text}', { text: title })} onClick={open}>
    {cover ? <MediaImage src={snsMediaUrl(cover, media.key, post.type)} label={title} /> : <span className="moment-tile-empty" aria-hidden="true" />}
    {video && <span className="moment-play" aria-hidden="true"><Play size={18} weight="fill" /></span>}
    {media.livePhoto && <span className="moment-badge">{t('实况')}</span>}
  </button>;
}

/** 经过服务端读取的图片；读不到（例如已经从微信的服务器上删除）时显示说明这一点的占位。 */
function MediaImage({ src, label }: { src: string; label: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="moment-tile-empty" title={t('已删除或无法读取')}><ImageBroken size={20} aria-label={t('{label}无法读取', { label })} /></span>;
  return <img src={src} alt="" loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}
