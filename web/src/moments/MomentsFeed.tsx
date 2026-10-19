// The Moments timeline (#202): posts newest first, a page at a time, each with its text, pictures, video, shared
// link, place, likes and comments. Pictures and videos come through `/api/sns/media`, which fetches them from
// WeChat's servers once and keeps them.
import { useEffect, useRef, useState } from 'react';
import { ArrowClockwise, CircleNotch, Heart, ImageBroken, LinkSimple, MapPin, Play } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getTimeline, isVideo, isWechatMedia, snsMediaUrl, SNS_PAGE, type SnsFilter, type SnsMedia, type SnsPost } from '../api/sns';
import { Avatar } from '../Avatar';
import type { Preview } from '../chat/MessageView';
import { dayOf, formatClock, formatDayHeading } from '../chat/messages';
import { MomentsSkeleton } from '../Skeleton';

type FeedState =
  | { status: 'loading' }
  | { status: 'ready'; posts: SnsPost[]; hasMore: boolean; more: boolean; error?: string }
  | { status: 'error'; message: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || '读取朋友圈失败';

/** Give it a `key` per server or account and filter, so a new one starts from the top. */
export function MomentsFeed({ filter, scroller, onPreview }: {
  filter: SnsFilter;
  /** The scrolling pane, to read the next page before its end is reached. */
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

  // the next page while the end of the list comes near
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

  if (state.status === 'loading') return <MomentsSkeleton label="正在读取朋友圈" />;
  if (state.status === 'error') {
    return <div className="pane-message" role="alert"><p>{state.message}</p>
      <button type="button" className="secondary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={14} />重试</button></div>;
  }
  if (!state.posts.length) {
    const narrowed = filter.keyword.trim() || filter.start || filter.end;
    return <div className="pane-message"><p>{narrowed ? '没有符合筛选条件的朋友圈。' : '本机没有这些朋友圈。'}</p></div>;
  }
  return <div className="moments" aria-label="朋友圈">
    {state.posts.map(post => <MomentPost key={post.id} post={post} onPreview={onPreview} />)}
    <div ref={end} className="moments-end">
      {state.error && <p role="alert">{state.error}</p>}
      {state.hasMore
        ? <button type="button" className="secondary-button" onClick={more} disabled={state.more}>{state.more ? <><CircleNotch size={14} className="spin" />正在读取</> : '加载更早的朋友圈'}</button>
        : <p>没有更早的朋友圈了</p>}
    </div>
  </div>;
}

function when(seconds: number): string {
  return formatDayHeading(dayOf(seconds)) + ' ' + formatClock(seconds);
}

/** A post's media: pictures and videos on WeChat's servers, and a shared link (any other address). */
function partsOf(post: SnsPost): { pictures: SnsMedia[]; link?: SnsMedia } {
  const media = post.media ?? [];
  return { pictures: media.filter(m => isWechatMedia(m.url)), link: media.find(m => m.url && !isWechatMedia(m.url)) };
}

export function MomentPost({ post, onPreview }: { post: SnsPost; onPreview(preview: Preview): void }) {
  const { pictures: shown, link } = partsOf(post);
  const place = post.location?.poiName || post.location?.city;
  const name = post.nickname || post.username;
  return <article className="moment" aria-label={name + '，' + when(post.createTime)}>
    <Avatar name={name} url={post.avatarUrl} />
    <div className="moment-body">
      <header><strong className="clamp-1">{name}</strong><time dateTime={new Date(post.createTime * 1000).toISOString()}>{when(post.createTime)}</time></header>
      {post.contentDesc?.trim() && <p className="moment-text">{post.contentDesc.trim()}</p>}
      {link && <a className="moment-link" href={link.url} target="_blank" rel="noreferrer noopener">
        {isWechatMedia(link.thumb) ? <MediaImage src={snsMediaUrl(link.thumb, link.key, post.type)} label="链接封面" /> : <span className="moment-link-icon" aria-hidden="true"><LinkSimple size={20} /></span>}
        <span className="clamp-2">{hostOf(link.url)}</span>
      </a>}
      {shown.length > 0 && <div className={'moment-media count-' + Math.min(shown.length, 9)}>
        {shown.slice(0, 9).map((m, i) => <MediaTile key={i} media={m} post={post} index={i} onPreview={onPreview} />)}
      </div>}
      {place && <p className="moment-place"><MapPin size={12} aria-hidden="true" />{place}</p>}
      {(post.likes?.length || post.comments?.length) ? <div className="moment-social">
        {post.likes?.length ? <p className="moment-likes"><Heart size={12} weight="fill" aria-label="赞" />{post.likes.join('，')}</p> : null}
        {post.comments?.length ? <ul className="moment-comments" aria-label="评论">
          {post.comments.map((c, i) => <li key={c.id || i}><b>{c.nickname}</b>{c.refNickname && <> 回复 <b>{c.refNickname}</b></>}：{c.content}</li>)}
        </ul> : null}
      </div> : null}
    </div>
  </article>;
}

const hostOf = (url: string) => { try { return new URL(url).hostname; } catch { return url; } };

function MediaTile({ media, post, index, onPreview }: { media: SnsMedia; post: SnsPost; index: number; onPreview(preview: Preview): void }) {
  const video = isVideo(media);
  const title = (post.nickname || post.username) + '的' + (video ? '视频' : '图片') + ' ' + (index + 1);
  const cover = isWechatMedia(media.thumb) ? media.thumb : !video && isWechatMedia(media.url) ? media.url : undefined;
  const open = () => {
    if (video) onPreview({ type: 'video', title, src: snsMediaUrl(media.url, media.key, post.type), poster: cover && snsMediaUrl(cover, media.key, post.type) });
    else onPreview({ type: 'image', title, src: snsMediaUrl(isWechatMedia(media.url) ? media.url : cover!, media.key, post.type) });
  };
  return <button type="button" className="moment-tile" aria-label={'预览' + title} onClick={open}>
    {cover ? <MediaImage src={snsMediaUrl(cover, media.key, post.type)} label={title} /> : <span className="moment-tile-empty" aria-hidden="true" />}
    {video && <span className="moment-play" aria-hidden="true"><Play size={18} weight="fill" /></span>}
    {media.livePhoto && <span className="moment-badge">实况</span>}
  </button>;
}

/** An image through the server; when it is gone from WeChat's servers, a placeholder that says so. */
function MediaImage({ src, label }: { src: string; label: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="moment-tile-empty" title="已删除或无法读取"><ImageBroken size={20} aria-label={label + '无法读取'} /></span>;
  return <img src={src} alt="" loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}
