// The resource browser (#206): the images, videos, voices and files of every conversation, newest first, in a grid
// grouped by month, narrowed by type (on the page) and by conversation and days (in a console card), a page at a time
// from the cursor of `chat resources` (#178) as the grid is scrolled. A picture or video opens in the preview, which goes through those read so far and
// can open the message in its conversation; a voice plays where it is; a file opens its message.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { ArrowSquareOut, CaretDown, ChatsCircle, CircleNotch, FilePdf, FileText, MagnifyingGlass, Warning, X } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import type { ChatSession } from '../api/chat';
import { listResources, voiceSeconds, type Resource, type ResourceQuery, type ResourceType } from '../api/details';
import { mediaUrl, type ParsedMessage } from '../api/messages';
import { Avatar } from '../Avatar';
import { DateRange } from '../calendar/DateRange';
import { formatClock, formatFileSize } from '../format';
import { imagePreview, PlayBadge, thumbFailure, Voice, type Preview } from '../MessageMedia';
import { Popover } from '../Popover';
import { Failed } from '../Failed';
import { Segmented } from '../Segmented';
import { MediaSkeleton } from '../Skeleton';
import { formatDate, locale, msg, t } from '../i18n';

const FILTERS: { label: string; types: ResourceType[] }[] = [
  { label: msg('全部'), types: ['image', 'video', 'voice', 'file'] },
  { label: msg('图片'), types: ['image'] },
  { label: msg('视频'), types: ['video'] },
  { label: msg('语音'), types: ['voice'] },
  { label: msg('文件'), types: ['file'] },
];

/** The conversation and days the console card narrows the page to. */
export interface ResourceFilter {
  session: string | null;
  /** `YYYY-MM-DD`, inclusive; empty for no bound. */
  from: string;
  to: string;
}

/** Every conversation, every day. */
export const ALL_RESOURCES: ResourceFilter = { session: null, from: '', to: '' };

const narrowed = (filter: ResourceFilter) => Boolean(filter.session || filter.from || filter.to);

/** What the filter narrows to, for a closed card's header: the conversation, by date, or 「全部」. */
export function resourceFilterSummary(filter: ResourceFilter, sessions: readonly ChatSession[]): string {
  const name = filter.session && (sessions.find(s => s.username === filter.session)?.displayName || filter.session);
  const parts = [name, (filter.from || filter.to) && t('按日期')].filter(Boolean);
  return parts.length ? parts.join(' · ') : t('全部');
}

/** Resources a page: a few screens of the grid. */
export const PAGE = 120;
/** Conversations listed in the picker before a search narrows them. */
const LISTED = 50;

type Pages =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; items: Resource[]; next: string | null; more: boolean; error?: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取资源失败');

/** The pages of `query` read so far, the next one read by `more`. */
function usePages(query: ResourceQuery): [Pages, () => void, () => void] {
  const key = JSON.stringify(query);
  const [state, setState] = useState<Pages>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const request = useRef<AbortController | null>(null);
  const current = useRef(state);
  current.current = state;
  useEffect(() => {
    const controller = new AbortController();
    request.current = controller;
    setState({ status: 'loading' });
    listResources(query, null, PAGE, controller.signal).then(
      page => { if (!controller.signal.aborted) setState({ status: 'ready', items: page.items, next: page.nextCursor, more: false }); },
      problem => { if (!controller.signal.aborted) setState({ status: 'error', message: errorOf(problem) }); },
    );
    return () => controller.abort();
    // the query is read again when its key changes
  }, [key, round]);
  const more = () => {
    const signal = request.current?.signal;
    const s = current.current;
    if (!signal || signal.aborted || s.status !== 'ready' || s.more || !s.next) return;
    const shown = s.items;
    setState({ ...s, more: true, error: undefined });
    listResources(query, s.next, PAGE, signal).then(
      page => { if (!signal.aborted) setState(old => (old.status === 'ready' && old.items === shown ? { ...old, items: [...shown, ...page.items], next: page.nextCursor, more: false } : old)); },
      problem => { if (!signal.aborted) setState(old => (old.status === 'ready' && old.items === shown ? { ...old, more: false, error: errorOf(problem) } : old)); },
    );
  };
  return [state, more, () => setRound(r => r + 1)];
}

const asMessage = (item: Resource) => ({ ...item, voiceDurationSeconds: voiceSeconds(item) }) as unknown as ParsedMessage;
const sessionOf = (item: Resource) => item.sessionId ?? '';
const nameOf = (item: Resource) => item.sessionDisplayName || item.sessionId || '';
const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString(locale(), { year: 'numeric', month: 'numeric', day: 'numeric' });
const month = (seconds: number) => formatDate(new Date(seconds * 1000), { year: 'numeric', month: 'long' });
const keyOf = (item: Resource) => item._cursor ?? sessionOf(item) + ':' + item.localId + '.' + item.createTime;

/** What a picture or video opens in the preview, with its way back to the conversation. */
function previewOf(item: Resource, onOpen: (item: Resource) => void): Preview | null {
  const message = asMessage(item);
  const locate = () => onOpen(item);
  const title = (item.resourceType === 'video' ? t('视频') : t('图片')) + ' · ' + nameOf(item);
  if (item.resourceType === 'image') return { ...imagePreview(sessionOf(item), message, title), locate };
  if (item.resourceType === 'video') return { type: 'video', src: mediaUrl('video-data', sessionOf(item), message), poster: mediaUrl('video-data', sessionOf(item), message, { thumb: true }), title, locate };
  return null;
}

/** A thumbnail, with `badge` over it; when it cannot be decoded, why (the server's answer: a format not supported yet,
 * a file not downloaded…), with the badge in place of the warning only when `video`, the video it stands for, still
 * plays, so that the badge never covers the reason. */
function Thumb({ src, label, video, badge }: { src: string; label: string; video?: string; badge?: ReactNode }) {
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading');
  const [failure, setFailure] = useState<{ reason: string; plays: boolean } | null>(null);
  useEffect(() => {
    if (state !== 'failed') return;
    let live = true;
    thumbFailure(src, video).then(f => { if (live) setFailure(f); });
    return () => { live = false; };
  }, [state, src, video]);
  if (state === 'failed') {
    return <span className="resource-missing">
      {failure?.plays ? badge : <Warning size={20} />}
      <small>{failure?.plays ? t('缩略图无法显示') : t('{label}无法显示', { label })}</small>
      {failure?.reason && <small className="clamp-2">{failure.reason}</small>}
    </span>;
  }
  return <><img className={state === 'loading' ? 'loading' : undefined} src={src} alt="" loading="lazy" decoding="async" onLoad={() => setState('ready')} onError={() => setState('failed')} />{badge}</>;
}

function Tile({ item, onPreview, onOpen }: { item: Resource; onPreview(item: Resource): void; onOpen(item: Resource): void }) {
  const sid = sessionOf(item);
  const message = asMessage(item);
  const when = day(item.createTime) + ' ' + formatClock(item.createTime);
  const caption = <span className="resource-caption">
    <span className="clamp-1">{nameOf(item)}</span><small className="num">{when}</small>
    {/* a voice opens its message from here; a picture or video from its preview, a file from the tile */}
    {item.resourceType === 'voice' && <button type="button" className="icon-button resource-locate" tabIndex={-1} aria-label={t('在会话中定位')} title={t('在会话中定位')} onClick={() => onOpen(item)}><ArrowSquareOut size={14} /></button>}
  </span>;
  switch (item.resourceType) {
    case 'image':
      return <div className="resource-tile"><button type="button" className="resource-open as-control press-on-hover resource-thumb" aria-label={t('预览图片，{name}，{when}', { name: nameOf(item), when })} onClick={() => onPreview(item)}>
        <Thumb src={mediaUrl('image-data', sid, message, { variant: 'thumb' })} label={t('图片')} />
      </button>{caption}</div>;
    case 'video':
      return <div className="resource-tile"><button type="button" className="resource-open as-control press-on-hover resource-thumb" aria-label={t('播放视频，{name}，{when}', { name: nameOf(item), when })} onClick={() => onPreview(item)}>
        <Thumb src={mediaUrl('video-data', sid, message, { thumb: true })} label={t('视频')} video={mediaUrl('video-data', sid, message)} badge={<PlayBadge size={18} />} />
      </button>{caption}</div>;
    case 'voice':
      return <div className="resource-tile"><Voice sessionId={sid} message={message} className="resource-open as-control press-on-hover resource-voice" />{caption}</div>;
    default:
      return <div className="resource-tile"><button type="button" className="resource-open as-control press-on-hover resource-file" aria-label={t('{file}，{name}，{when}，在会话中定位', { file: item.fileName || t('文件'), name: nameOf(item), when })} onClick={() => onOpen(item)}>
        {/^pdf$/i.test(item.fileExt ?? '') ? <FilePdf size={32} /> : <FileText size={32} />}
        <span className="clamp-2">{item.fileName || t('文件')}</span><small className="num">{formatFileSize(item.fileSize)}</small>
      </button>{caption}</div>;
  }
}

/** The conversation to narrow to, or all of them: those with the latest messages first, found by name. */
function SessionPicker({ sessions, selected, onSelect }: { sessions: readonly ChatSession[]; selected: string | null; onSelect(username: string | null): void }) {
  const [open, setOpen] = useState(false);
  const [keyword, setKeyword] = useState('');
  const anchor = useRef<HTMLButtonElement>(null);
  const chosen = sessions.find(s => s.username === selected);
  const found = useMemo(() => {
    const words = keyword.trim().toLowerCase();
    const list = sessions.filter(s => s.sortTimestamp > 0);
    return words ? list.filter(s => (s.displayName || s.username).toLowerCase().includes(words)) : list;
  }, [sessions, keyword]);
  const close = (focus: boolean) => { if (focus) anchor.current?.focus(); setOpen(false); };
  const pick = (username: string | null) => { onSelect(username); close(true); };
  return <>
    <button ref={anchor} type="button" className={'group-picker as-control' + (open ? ' on' : '')} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
      {chosen ? <Avatar name={chosen.displayName || chosen.username} url={chosen.avatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true"><ChatsCircle size={20} /></span>}
      <span className="rank-copy"><strong className="clamp-1">{chosen ? chosen.displayName || chosen.username : selected ?? t('全部会话')}</strong>
        <span className="cache-text">{selected ? t('只看这个会话') : t('所有会话的资源')}</span></span>
      <CaretDown size={14} aria-hidden="true" />
    </button>
    <Popover anchor={anchor} open={open} onClose={reason => close(reason === 'escape')} label={t('选择会话')} className="group-popover">
      <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
        <input type="search" autoFocus aria-label={t('搜索会话')} placeholder={t('搜索会话名称')} value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
      <div className="group-options" role="listbox" data-roving="vertical" data-roving-items="[role=option]" aria-label={t('会话')}>
        {!keyword.trim() && <button type="button" role="option" aria-selected={!selected} className={'conversation-row' + (!selected ? ' active' : '')} onClick={() => pick(null)}>
          <span className="avatar avatar-initial" aria-hidden="true"><ChatsCircle size={20} /></span>
          <span className="conversation-copy"><span className="conversation-name clamp-1">{t('全部会话')}</span></span>
        </button>}
        {found.slice(0, LISTED).map(s => <button type="button" key={s.username} role="option" aria-selected={s.username === selected}
          className={'conversation-row' + (s.username === selected ? ' active' : '')} onClick={() => pick(s.username)}>
          <Avatar name={s.displayName || s.username} url={s.avatarUrl} />
          <span className="conversation-copy"><span className="conversation-name clamp-1">{s.displayName || s.username}</span></span>
        </button>)}
        {!found.length && <p className="card-note">{t('没有找到。')}</p>}
      </div>
      {found.length > LISTED && <p className="card-note">{t('还有 {value} 个，搜索名称找到它。', { value: found.length - LISTED })}</p>}
    </Popover>
  </>;
}

/** The console card: the conversation and the days. */
export function ResourceFilterCard({ sessions, filter, onChange }: { sessions: readonly ChatSession[]; filter: ResourceFilter; onChange(filter: ResourceFilter): void }) {
  return <div className="resource-filter">
    <SessionPicker sessions={sessions} selected={filter.session} onSelect={session => onChange({ ...filter, session })} />
    <div className="search-date-row"><span className="field-label">{t('时间范围')}</span>
      <DateRange label={t('资源的时间范围')} from={filter.from} to={filter.to} onChange={range => onChange({ ...filter, ...range })} /></div>
    {narrowed(filter) && <button type="button" className="text-button moments-clear" onClick={() => onChange(ALL_RESOURCES)}><X size={12} />{t('清除筛选')}</button>}
  </div>;
}

/** The images, videos, voices and files the filter narrows to, newest first, a page at a time. */
export function ResourcesPage({ filter, scroller, onPreview, onOpen }: {
  filter: ResourceFilter;
  /** The page's scroll area (`ScrollPage`), which the grid scrolls in. */
  scroller: RefObject<HTMLElement | null>;
  onPreview(preview: Preview): void;
  onOpen(sessionId: string, localId: number, createTime: number): void;
}) {
  const [kind, setKind] = useState(0);
  const query = useMemo<ResourceQuery>(() => ({ ...filter, types: FILTERS[kind].types }), [filter, kind]);
  const [state, more, retry] = usePages(query);
  const open = (item: Resource) => onOpen(sessionOf(item), item.localId, item.createTime);
  const items = state.status === 'ready' ? state.items : [];
  const media = useMemo(() => items.filter(item => item.resourceType === 'image' || item.resourceType === 'video'), [items]);
  const preview = (item: Resource) => {
    const own = previewOf(item, open);
    if (!own) return;
    const index = media.indexOf(item);
    onPreview(media.length > 1 && index >= 0 ? { ...own, gallery: { items: media.map(m => previewOf(m, open)!), index } } : own);
  };

  // the next page as the end of the grid comes near: within 800px of the bottom of the page's scroll area, or of the
  // window where the area grows with the grid and the document scrolls instead (a phone)
  const end = useRef<HTMLDivElement>(null);
  const moreRef = useRef(more);
  moreRef.current = more;
  const next = state.status === 'ready' ? state.next : null;
  useEffect(() => {
    const target = end.current;
    if (!target || !next || typeof IntersectionObserver === 'undefined') return;
    const area = scroller.current;
    const root = area && area.scrollHeight > area.clientHeight ? area : null;
    const watch = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) moreRef.current(); }, { root, rootMargin: '0px 0px 800px 0px' });
    watch.observe(target);
    return () => watch.disconnect();
  }, [next, scroller]);

  let lastMonth = '';
  return <div className="resources">
    <Segmented className="resource-types" count={FILTERS.length} index={kind} role="group" aria-label={t('资源类型')}>
      {FILTERS.map((f, i) => <button type="button" key={f.label} aria-pressed={kind === i} onClick={() => setKind(i)}>{t(f.label)}</button>)}
    </Segmented>
    {state.status === 'loading' && <MediaSkeleton label={t('正在读取资源')} items={8} />}
    {state.status === 'error' && <Failed title={t('读取资源失败')} message={state.message} onRetry={retry} />}
    {state.status === 'ready' && (items.length
      ? <div className="resource-grid" data-roving="grid" data-roving-items=".resource-open">
        {items.map(item => {
          const m = month(item.createTime);
          const heading = m !== lastMonth ? <h2 className="resource-month" key={'m' + m}>{m}</h2> : null;
          lastMonth = m;
          return [heading, <Tile key={keyOf(item)} item={item} onPreview={preview} onOpen={open} />];
        })}
      </div>
      : <p className="empty-text">{narrowed(filter)
        ? kind ? t('没有符合筛选条件的{kind}。', { kind: t(FILTERS[kind].label) }) : t('没有符合筛选条件的资源。')
        : kind ? t('没有找到{kind}。', { kind: t(FILTERS[kind].label) }) : t('没有找到图片、视频、语音或文件。')}</p>)}
    {state.status === 'ready' && <div ref={end} className="resource-end">
      {state.error && <p className="empty-text" role="alert">{state.error}</p>}
      {state.next && <button type="button" className="secondary-button" onClick={more} disabled={state.more}>{state.more ? <><CircleNotch size={14} className="spin" />{t('正在读取')}</> : t('加载更多')}</button>}
    </div>}
  </div>;
}
