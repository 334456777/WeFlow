// 操作控制台的「媒体文件」卡片（#200）：会话里的图片、视频和文件，最新的在前，一次读一页（`chat resources`）；
// 点一项就打开会话并定位到那条消息（与搜索结果一样）。
import { useEffect, useRef, useState } from 'react';
import { ArrowSquareOut, CircleNotch, FileText, VideoCamera } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getResources, RESOURCE_PAGE, type Resource, type ResourcePage, type ResourceType } from '../api/details';
import { useGathered, useLive } from '../api/live';
import { mediaUrl, type ParsedMessage } from '../api/messages';
import { HoverTrack } from '../HoverTrack';
import { Segmented } from '../Segmented';
import { formatClock } from '../format';
import { MediaSkeleton } from '../Skeleton';
import { locale, msg, t } from '../i18n';

const FILTERS: { label: string; types: ResourceType[] }[] = [
  { label: msg('全部'), types: ['image', 'video', 'file'] },
  { label: msg('图片'), types: ['image'] },
  { label: msg('视频'), types: ['video'] },
  { label: msg('文件'), types: ['file'] },
];

type State = { status: 'loading' } | { status: 'ready'; filter: number; items: Resource[]; next: string | null; more: boolean; error?: string } | { status: 'error'; message: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取媒体文件失败');

function size(bytes?: number): string {
  if (!bytes) return '';
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / 1024 / 1024).toFixed(1) + ' MB';
}

const keyOf = (item: Resource) => item.resourceType + item.localId + '.' + item.createTime;

function unseen(items: readonly Resource[], shown: readonly Resource[]): Resource[] {
  const seen = new Set(shown.map(keyOf));
  return items.filter(item => !seen.has(keyOf(item)));
}

/**
 * 合并有新消息或重新展开卡片时重读的第一页：它取代自己覆盖的那一段（撤回的消息随之消失），已经显示的更早的页留下，
 * 因此来了新消息时列表不会退回第一页。换了筛选或这一页不满一页时，只显示这一页。
 */
function withFirstPage(old: State, filter: number, page: ResourcePage): State {
  const oldest = page.items.at(-1);
  if (old.status !== 'ready' || old.filter !== filter || !oldest || page.items.length < RESOURCE_PAGE) {
    return { status: 'ready', filter, items: page.items, next: page.nextCursor, more: false };
  }
  const rest = unseen(old.items.filter(item => item.createTime <= oldest.createTime), page.items);
  return { status: 'ready', filter, items: [...page.items, ...rest], next: rest.length ? old.next : page.nextCursor, more: false };
}

const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString(locale(), { year: 'numeric', month: 'numeric', day: 'numeric' });

/** 一个会话的媒体，`active`（卡片展开）时才读取；每个会话要给它不同的 `key`。 */
export function MediaPanel({ sessionId, active = true, onOpen }: { sessionId: string; active?: boolean; onOpen: (item: Resource) => void }) {
  const [filter, setFilter] = useState(0);
  const [state, setState] = useState<State>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const request = useRef<AbortController | null>(null);
  const types = FILTERS[filter].types;

  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    request.current = controller;
    // 同一个筛选重读时，这期间保留已显示的内容
    setState(old => (old.status === 'ready' && old.filter === filter ? old : { status: 'loading' }));
    getResources(sessionId, FILTERS[filter].types, null, controller.signal).then(
      page => { if (!controller.signal.aborted) setState(old => withFirstPage(old, filter, page)); },
      problem => { if (!controller.signal.aborted) setState({ status: 'error', message: errorOf(problem) }); },
    );
    return () => controller.abort();
  }, [sessionId, filter, round, active]);

  // 刚发的图片或文件出现在最上面（#201）；连续的消息合并成一次读取
  const refresh = useGathered(() => setRound(n => n + 1));
  useLive(event => {
    if (event.type === 'resumed' || event.sessionId === sessionId) refresh();
  });

  const more = () => {
    const signal = request.current?.signal;
    if (!active || signal?.aborted || state.status !== 'ready' || state.more || !state.next) return;
    const shown = state.items;
    setState({ ...state, more: true, error: undefined });
    // 从最后显示的一页的游标接着读下一页：这期间发来的消息不会让它错位（#178）
    getResources(sessionId, types, state.next, signal).then(
      page => { if (!signal?.aborted) setState(old => (old.status === 'ready' && old.items === shown
        ? { ...old, items: [...shown, ...unseen(page.items, shown)], next: page.nextCursor, more: false }
        : old)); },
      problem => { if (!signal?.aborted) setState(old => (old.status === 'ready' && old.items === shown ? { ...old, more: false, error: errorOf(problem) } : old)); },
    );
  };

  return <div className="media-panel">
    <Segmented className="media-filter" count={FILTERS.length} index={filter} role="group" aria-label={t('媒体类型')}>
      {FILTERS.map((f, i) => <button type="button" key={f.label} aria-pressed={filter === i} onClick={() => setFilter(i)}>{t(f.label)}</button>)}
    </Segmented>
    {state.status === 'loading' && <MediaSkeleton label={t('正在读取媒体')} />}
    {state.status === 'error' && <p className="empty-text" role="alert">{state.message} <button type="button" className="text-button" onClick={() => setRound(n => n + 1)}>{t('重试')}</button></p>}
    {state.status === 'ready' && (state.items.length
      ? <div className="media-list" data-roving="grid" data-roving-items=".media-item"><HoverTrack items=".media-item" />
        {state.items.map(item => <MediaItem key={keyOf(item)} sessionId={sessionId} item={item} onOpen={onOpen} />)}
        {state.next && <button type="button" className="secondary-button media-more" onClick={more} disabled={state.more}>{state.more ? <><CircleNotch size={14} className="spin" />{t('正在读取')}</> : t('加载更多')}</button>}
        {state.error && <p className="empty-text" role="alert">{state.error}</p>}
      </div>
      : <p className="empty-text">{filter ? t('这个会话没有这类媒体。') : t('这个会话没有图片、视频或文件。')}</p>)}
  </div>;
}

function MediaItem({ sessionId, item, onOpen }: { sessionId: string; item: Resource; onOpen: (item: Resource) => void }) {
  const [broken, setBroken] = useState(false);
  const label = item.resourceType === 'file' ? item.fileName || t('文件') : item.resourceType === 'video' ? t('视频') : t('图片');
  const detail = [day(item.createTime) + ' ' + formatClock(item.createTime), item.resourceType === 'file' ? size(item.fileSize) : ''].filter(Boolean).join(' · ');
  const thumb = item.resourceType === 'image' && !broken
    ? <img src={mediaUrl('image-data', sessionId, item as unknown as ParsedMessage, { variant: 'thumb' })} alt="" loading="lazy" decoding="async" onError={() => setBroken(true)} />
    : <span className="media-icon" aria-hidden="true">{item.resourceType === 'video' ? <VideoCamera size={24} /> : <FileText size={24} />}</span>;
  return <button className="media-item as-row" type="button" aria-label={t('{label}，{detail}，在会话中定位', { label, detail })} onClick={() => onOpen(item)}>
    {thumb}<span><span className="clamp-1">{label}</span><small>{detail}</small></span><ArrowSquareOut size={14} />
  </button>;
}
