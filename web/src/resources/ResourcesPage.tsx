// 资源浏览（#206）：全部会话的图片、视频、语音和文件，最新的在前，按月份分组排成网格；在页面上按类型筛选，
// 在操作控制台的卡片里按会话和日期筛选。滚动网格时从 `chat resources` 的游标（#178）接着一页一页地读。
// 图片和视频在预览里打开，预览可以在已读到的图片和视频之间切换，也可以打开会话里的那条消息；
// 语音就地播放；点文件打开它所在的消息。
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

/** 操作控制台的卡片用来筛选页面的会话和日期。 */
export interface ResourceFilter {
  session: string | null;
  /** `YYYY-MM-DD`，包含这一天；为空表示不限。 */
  from: string;
  to: string;
}

/** 不筛选：全部会话、全部日期。 */
export const ALL_RESOURCES: ResourceFilter = { session: null, from: '', to: '' };

const narrowed = (filter: ResourceFilter) => Boolean(filter.session || filter.from || filter.to);

/** 卡片收起时标题栏上的筛选摘要：会话和 `按日期`，都没有时为 `全部`。 */
export function resourceFilterSummary(filter: ResourceFilter, sessions: readonly ChatSession[]): string {
  const name = filter.session && (sessions.find(s => s.username === filter.session)?.displayName || filter.session);
  const parts = [name, (filter.from || filter.to) && t('按日期')].filter(Boolean);
  return parts.length ? parts.join(' · ') : t('全部');
}

/** 每页的资源数，约为网格的几屏。 */
export const PAGE = 120;
/** 选择器最多列出的会话数，其余的靠搜索找到。 */
const LISTED = 50;

type Pages =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; items: Resource[]; next: string | null; more: boolean; error?: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取资源失败');

/** `query` 已经读到的各页，读取下一页的 `more`，以及从第一页重读的函数。 */
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
    // `query` 的键（`JSON.stringify` 的结果）变化时才重读
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

/** 图片或视频在预览里打开的内容，带着回到会话里那条消息的入口（`locate`）；其他资源为 null。 */
function previewOf(item: Resource, onOpen: (item: Resource) => void): Preview | null {
  const message = asMessage(item);
  const locate = () => onOpen(item);
  const title = (item.resourceType === 'video' ? t('视频') : t('图片')) + ' · ' + nameOf(item);
  if (item.resourceType === 'image') return { ...imagePreview(sessionOf(item), message, title), locate };
  if (item.resourceType === 'video') return { type: 'video', src: mediaUrl('video-data', sessionOf(item), message), poster: mediaUrl('video-data', sessionOf(item), message, { thumb: true }), title, locate };
  return null;
}

/**
 * 缩略图，上面叠着 `badge`。解码失败时显示原因（服务端的回答，如不支持的格式、没有下载的文件）；
 * 只有它代表的视频 `video` 仍能播放时，才用 `badge` 代替警告图标，这样 `badge` 不会盖住原因。
 */
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
    {/* 语音从这里打开它所在的消息；图片和视频从预览里打开，文件点格子本身打开 */}
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

/** 选要筛选的会话或者全部会话：按会话列表的顺序列出（不列出 `sortTimestamp` 为 0 的），可以按名称搜索。 */
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

/** 操作控制台的筛选卡片：会话和日期范围。 */
export function ResourceFilterCard({ sessions, filter, onChange }: { sessions: readonly ChatSession[]; filter: ResourceFilter; onChange(filter: ResourceFilter): void }) {
  return <div className="resource-filter">
    <SessionPicker sessions={sessions} selected={filter.session} onSelect={session => onChange({ ...filter, session })} />
    <div className="search-date-row"><span className="field-label">{t('时间范围')}</span>
      <DateRange label={t('资源的时间范围')} from={filter.from} to={filter.to} onChange={range => onChange({ ...filter, ...range })} /></div>
    {narrowed(filter) && <button type="button" className="text-button moments-clear" onClick={() => onChange(ALL_RESOURCES)}><X size={12} />{t('清除筛选')}</button>}
  </div>;
}

/** 按筛选条件列出的图片、视频、语音和文件，最新的在前，一次读一页。 */
export function ResourcesPage({ filter, scroller, onPreview, onOpen }: {
  filter: ResourceFilter;
  /** 页面的滚动区域（`ScrollPage`），网格在里面滚动。 */
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

  // 网格的末尾接近时读下一页：距页面滚动区域的底部 800px 以内；滚动区域随网格变高、
  // 改由整个文档滚动时（手机上），按窗口的底部计算
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
