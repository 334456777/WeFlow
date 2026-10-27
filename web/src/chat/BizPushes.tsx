// An official account's pushes (#203): not chat bubbles but a feed of article cards, newest first, from
// `biz messages`; a card opens its article. WeChat for PC may keep no push of an account on this machine (#113):
// then it says so.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Article, ArrowClockwise, CircleNotch, Image as ImageIcon } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { BIZ_PAGE, getBizPushes, type BizArticle, type BizPush } from '../api/biz';
import type { ChatSession } from '../api/chat';
import { useLive } from '../api/live';
import { Avatar } from '../Avatar';
import { dayOf, formatClock, formatDayHeading } from './messages';
import { MomentsSkeleton } from '../Skeleton';

interface Feed {
  status: 'loading' | 'ready' | 'error';
  pushes: BizPush[];
  hasMore: boolean;
  loadingMore: boolean;
  error?: string;
}

const LOADING: Feed = { status: 'loading', pushes: [], hasMore: false, loadingMore: false };

const messageOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || '读取推送失败';

function Cover({ src, className }: { src: string; className: string }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) return <span className={className + ' biz-cover-missing'} aria-hidden="true"><ImageIcon size={20} /></span>;
  return <img className={className} src={src} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} />;
}

/** A link to an article, or plain text when the push has no address. */
function ArticleLink({ article, className, children }: { article: Pick<BizArticle, 'url' | 'title'>; className: string; children: ReactNode }) {
  return article.url
    ? <a className={className} href={article.url} target="_blank" rel="noreferrer noopener" aria-label={article.title || '打开文章'}>{children}</a>
    : <div className={className}>{children}</div>;
}

function PushCard({ push }: { push: BizPush }) {
  // a row that does not say when (the contract's null) shows as the start of 1970 rather than breaking the list
  const seconds = push.createTime ?? 0;
  const time = new Date(seconds * 1000);
  // the first article leads; the others of the same push follow as rows
  const more = push.contentList.filter((item, index) => !(index === 0 && item.url === push.url && item.title === push.title));
  return <article className="biz-push">
    <time className="biz-push-time" dateTime={time.toISOString()}>{formatDayHeading(dayOf(seconds), new Date())} {formatClock(seconds)}</time>
    <ArticleLink article={push} className="biz-lead">
      <Cover src={push.cover} className="biz-lead-cover" />
      <strong className="biz-lead-title clamp-2">{push.title || '无标题'}</strong>
      {push.description && <span className="biz-lead-summary clamp-2">{push.description}</span>}
    </ArticleLink>
    {more.map((item, index) => <ArticleLink key={index} article={item} className="biz-item">
      <span className="biz-item-title clamp-2">{item.title || '无标题'}</span>
      <Cover src={item.cover} className="biz-item-cover" />
    </ArticleLink>)}
  </article>;
}

export function BizPushes({ session }: { session: ChatSession }) {
  const id = session.username;
  const [feed, setFeed] = useState<Feed>(LOADING);
  const [round, setRound] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    setFeed(LOADING);
    getBizPushes(id, 0, controller.signal).then(pushes => {
      setFeed({ status: 'ready', pushes, hasMore: pushes.length === BIZ_PAGE, loadingMore: false });
    }, problem => {
      if (!controller.signal.aborted) setFeed({ ...LOADING, status: 'error', error: messageOf(problem) });
    });
    return () => controller.abort();
  }, [id, round]);

  const latest = useRef(feed);
  latest.current = feed;
  const more = useCallback(() => {
    const now = latest.current;
    if (now.status !== 'ready' || !now.hasMore || now.loadingMore) return;
    setFeed({ ...now, loadingMore: true, error: undefined });
    getBizPushes(id, now.pushes.length).then(got => setFeed(old => {
      const seen = new Set(old.pushes.map(p => p.localId + ':' + p.createTime));
      const added = got.filter(p => !seen.has(p.localId + ':' + p.createTime));
      return { ...old, pushes: [...old.pushes, ...added], hasMore: got.length === BIZ_PAGE, loadingMore: false };
    }), problem => setFeed(old => ({ ...old, loadingMore: false, error: messageOf(problem) })));
  }, [id]);

  // a new push of this account, or a break: read the newest again (#201)
  useLive(event => {
    if (event.type === 'resumed' || event.sessionId === id) setRound(n => n + 1);
  });

  if (feed.status === 'loading') {
    return <MomentsSkeleton label="正在读取推送" />;
  }
  if (feed.status === 'error') {
    return <div className="pane-message" role="alert"><p>{feed.error}</p><button type="button" className="secondary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={14} />重试</button></div>;
  }
  return <div className="biz-feed">
    <header className="biz-head">
      <Avatar name={session.displayName || id} url={session.avatarUrl} />
      <div><h2>{session.displayName || id}</h2><p><Article size={12} />公众号 · 最新的推送在最上面</p></div>
    </header>
    {feed.pushes.length
      ? feed.pushes.map(push => <PushCard key={push.localId + ':' + push.createTime} push={push} />)
      : <p className="biz-empty">本机没有这个公众号的推送记录。公众号历史文章可能在网页中显示；已删除或尚未保存在本机的消息无法读取。</p>}
    {feed.error && <p className="biz-empty" role="alert">{feed.error}</p>}
    {feed.hasMore && <button type="button" className="secondary-button biz-more" onClick={more} disabled={feed.loadingMore}>
      {feed.loadingMore ? <><CircleNotch size={14} className="spin" />正在加载</> : '加载更早的推送'}
    </button>}
  </div>;
}
