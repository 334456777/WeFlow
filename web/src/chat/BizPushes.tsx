// 公众号的推送（#203）：不用聊天气泡，而是 `biz messages` 读到的文章卡片列表，最新的在最上面；点卡片打开文章。
// 微信电脑版可能没有在本机保存某个公众号的任何推送（#113），这时显示说明。
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Article, ArrowClockwise, CircleNotch, Image as ImageIcon } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { BIZ_PAGE, getBizPushes, type BizArticle, type BizPush } from '../api/biz';
import type { ChatSession } from '../api/chat';
import { LIVE_READ_MS, useGathered, useLive } from '../api/live';
import { Avatar } from '../Avatar';
import { dayOf, formatClock, formatDayHeading } from '../format';
import { MomentsSkeleton } from '../Skeleton';
import { t } from '../i18n';
import { safeUrl } from '../links';

interface Feed {
  status: 'loading' | 'ready' | 'error';
  pushes: BizPush[];
  hasMore: boolean;
  loadingMore: boolean;
  error?: string;
}

const LOADING: Feed = { status: 'loading', pushes: [], hasMore: false, loadingMore: false };

const messageOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取推送失败');

function Cover({ src, className }: { src: string; className: string }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) return <span className={className + ' biz-cover-missing'} aria-hidden="true"><ImageIcon size={20} /></span>;
  return <img className={className} src={src} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} />;
}

/** 打开文章的链接；推送没有网页地址时是不能点击的普通元素。 */
function ArticleLink({ article, className, children }: { article: Pick<BizArticle, 'url' | 'title'>; className: string; children: ReactNode }) {
  const url = safeUrl(article.url);
  return url
    ? <a className={className + ' as-row'} href={url} target="_blank" rel="noreferrer noopener" aria-label={article.title || t('打开文章')}>{children}</a>
    : <div className={className}>{children}</div>;
}

function PushCard({ push }: { push: BizPush }) {
  // 没有时间的行（契约里的 null）显示为 1970 年初，不让整个列表出错
  const seconds = push.createTime ?? 0;
  const time = new Date(seconds * 1000);
  // 推送本身的文章作为头条；同一次推送的其他文章在下面各占一行，`contentList` 的第一项与头条相同时不重复显示
  const more = push.contentList.filter((item, index) => !(index === 0 && item.url === push.url && item.title === push.title));
  return <article className="biz-push">
    <time className="biz-push-time" dateTime={time.toISOString()}>{formatDayHeading(dayOf(seconds), new Date())} {formatClock(seconds)}</time>
    <ArticleLink article={push} className="biz-lead">
      <Cover src={push.cover} className="biz-lead-cover" />
      <strong className="biz-lead-title clamp-2">{push.title || t('无标题')}</strong>
      {push.description && <span className="biz-lead-summary clamp-2">{push.description}</span>}
    </ArticleLink>
    {more.map((item, index) => <ArticleLink key={index} article={item} className="biz-item">
      <span className="biz-item-title clamp-2">{item.title || t('无标题')}</span>
      <Cover src={item.cover} className="biz-item-cover" />
    </ArticleLink>)}
  </article>;
}

const keyOf = (push: BizPush) => push.localId + ':' + push.createTime;

/**
 * 合并有新推送或中断恢复后重读的第一页：它取代自己覆盖的那一段（公众号里已经没有的推送随之消失），
 * 已经显示的更早的推送留在它下面，因此来了新推送时列表不会退回第一页。这一页不满一页（就是全部推送），
 * 或者与已显示的推送没有重叠（这期间来的推送比一页还多）时，只显示这一页，否则中间会缺少推送。
 */
function withFirstPage(old: Feed, pushes: BizPush[]): Feed {
  const fresh: Feed = { status: 'ready', pushes, hasMore: pushes.length === BIZ_PAGE, loadingMore: false };
  const oldest = pushes.at(-1);
  if (old.status !== 'ready' || !oldest || pushes.length < BIZ_PAGE) return fresh;
  const page = new Set(pushes.map(keyOf));
  if (!old.pushes.some(push => page.has(keyOf(push)))) return fresh;
  const until = oldest.createTime ?? 0;
  const rest = old.pushes.filter(push => (push.createTime ?? 0) <= until && !page.has(keyOf(push)));
  return { ...fresh, pushes: [...pushes, ...rest], hasMore: rest.length ? old.hasMore : fresh.hasMore };
}

/** 公众号 `session` 的推送，一次读一页。 */
export function BizPushes({ session }: { session: ChatSession }) {
  const id = session.username;
  const [feed, setFeed] = useState<Feed>(LOADING);
  const [round, setRound] = useState(0);
  // 正在读取的更早的一页，它的偏移量按发出请求时的列表计算
  const paging = useRef<AbortController | null>(null);
  useEffect(() => () => paging.current?.abort(), []);

  useEffect(() => {
    const controller = new AbortController();
    // 重读：这期间保留已显示的内容
    setFeed(old => (old.status === 'ready' ? old : LOADING));
    getBizPushes(id, 0, controller.signal).then(pushes => {
      if (controller.signal.aborted) return;
      // 还没回来的更早一页是按旧列表的偏移量请求的，取消它
      paging.current?.abort();
      setFeed(old => withFirstPage(old, pushes));
    }, problem => {
      // 重读失败时保留已显示的列表
      if (!controller.signal.aborted) setFeed(old => (old.status === 'ready' ? old : { ...LOADING, status: 'error', error: messageOf(problem) }));
    });
    return () => controller.abort();
  }, [id, round]);

  const latest = useRef(feed);
  latest.current = feed;
  const more = useCallback(() => {
    const now = latest.current;
    if (now.status !== 'ready' || !now.hasMore || now.loadingMore) return;
    const controller = new AbortController();
    paging.current = controller;
    const shown = now.pushes;
    setFeed({ ...now, loadingMore: true, error: undefined });
    getBizPushes(id, shown.length, controller.signal).then(got => {
      if (controller.signal.aborted) return;
      setFeed(old => {
        if (old.pushes !== shown) return { ...old, loadingMore: false };
        const seen = new Set(shown.map(keyOf));
        const added = got.filter(push => !seen.has(keyOf(push)));
        return { ...old, pushes: [...shown, ...added], hasMore: got.length === BIZ_PAGE, loadingMore: false };
      });
    }, problem => {
      if (!controller.signal.aborted) setFeed(old => ({ ...old, loadingMore: false, error: messageOf(problem) }));
    });
  }, [id]);

  // 这个公众号有新推送，或者中断后恢复：重读最新的一页（#201），连续的事件合并成一次读取
  const refresh = useGathered(() => setRound(n => n + 1), LIVE_READ_MS);
  useLive(event => {
    if (event.type === 'resumed' || event.sessionId === id) refresh();
  });

  if (feed.status === 'loading') {
    return <MomentsSkeleton label={t('正在读取推送')} />;
  }
  if (feed.status === 'error') {
    return <div className="pane-message" role="alert"><p>{feed.error}</p><button type="button" className="secondary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  }
  return <div className="biz-feed">
    <header className="biz-head">
      <Avatar name={session.displayName || id} url={session.avatarUrl} />
      <div><h2>{session.displayName || id}</h2><p><Article size={12} />{t('公众号 · 最新的推送在最上面')}</p></div>
    </header>
    {feed.pushes.length
      ? feed.pushes.map(push => <PushCard key={keyOf(push)} push={push} />)
      : <p className="biz-empty">{t('本机没有这个公众号的推送记录。公众号历史文章可能在网页中显示；已删除或尚未保存在本机的消息无法读取。')}</p>}
    {feed.error && <p className="biz-empty" role="alert">{feed.error}</p>}
    {feed.hasMore && <button type="button" className="secondary-button biz-more" onClick={more} disabled={feed.loadingMore}>
      {feed.loadingMore ? <><CircleNotch size={14} className="spin" />{t('正在加载')}</> : t('加载更早的推送')}
    </button>}
  </div>;
}
