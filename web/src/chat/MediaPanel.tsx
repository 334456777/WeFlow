// The console's 「媒体文件」 for real conversations (#200): its images, videos and files, newest first, a page at a
// time (`chat resources`); one opens its conversation at the message (as a search hit does).
import { useEffect, useRef, useState } from 'react';
import { ArrowSquareOut, CircleNotch, FileText, VideoCamera } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getResources, RESOURCE_PAGE, type Resource, type ResourceType } from '../api/details';
import { useGathered, useLive } from '../api/live';
import { mediaUrl, type ParsedMessage } from '../api/messages';
import { HoverTrack } from '../HoverTrack';
import { Segmented } from '../Segmented';
import { formatClock } from './messages';

const FILTERS: { label: string; types: ResourceType[] }[] = [
  { label: '全部', types: ['image', 'video', 'file'] },
  { label: '图片', types: ['image'] },
  { label: '视频', types: ['video'] },
  { label: '文件', types: ['file'] },
];

type State = { status: 'loading' } | { status: 'ready'; filter: number; items: Resource[]; hasMore: boolean; more: boolean; error?: string } | { status: 'error'; message: string };

const errorOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || '读取媒体文件失败';

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
 * The first page, read again for a new message or on reopening the card: it takes the place of what it covers
 * (so a recalled message goes), and the older pages already shown stay, so a new message does not take the list
 * back to its first page.
 */
function withFirstPage(old: State, filter: number, page: { items: Resource[]; hasMore: boolean }): State {
  const oldest = page.items.at(-1);
  if (old.status !== 'ready' || old.filter !== filter || !oldest || page.items.length < RESOURCE_PAGE) {
    return { status: 'ready', filter, items: page.items, hasMore: page.hasMore, more: false };
  }
  const rest = unseen(old.items.filter(item => item.createTime <= oldest.createTime), page.items);
  return { status: 'ready', filter, items: [...page.items, ...rest], hasMore: rest.length ? old.hasMore : page.hasMore, more: false };
}

const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString('zh-CN', { year: 'numeric', month: 'numeric', day: 'numeric' });

/** One conversation's media, read while `active` (its card open): give it a `key` per conversation. */
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
    // read again for the same filter: what is shown stays meanwhile
    setState(old => (old.status === 'ready' && old.filter === filter ? old : { status: 'loading' }));
    getResources(sessionId, FILTERS[filter].types, 0, controller.signal).then(
      page => { if (!controller.signal.aborted) setState(old => withFirstPage(old, filter, page)); },
      problem => { if (!controller.signal.aborted) setState({ status: 'error', message: errorOf(problem) }); },
    );
    return () => controller.abort();
  }, [sessionId, filter, round, active]);

  // a picture or a file sent now shows at the top (#201); a burst of messages reads once, as each read is a command
  const refresh = useGathered(() => setRound(n => n + 1));
  useLive(event => {
    if (event.type === 'resumed' || event.sessionId === sessionId) refresh();
  });

  const more = () => {
    const signal = request.current?.signal;
    if (!active || signal?.aborted || state.status !== 'ready' || state.more) return;
    const shown = state.items;
    setState({ ...state, more: true, error: undefined });
    getResources(sessionId, types, shown.length, signal).then(
      page => { if (!signal?.aborted) setState(old => (old.status === 'ready' && old.items === shown
        // a message sent meanwhile moves the offsets on: what is already shown is not shown twice
        ? { ...old, items: [...shown, ...unseen(page.items, shown)], hasMore: page.hasMore && page.items.length === RESOURCE_PAGE, more: false }
        : old)); },
      problem => { if (!signal?.aborted) setState(old => (old.status === 'ready' && old.items === shown ? { ...old, more: false, error: errorOf(problem) } : old)); },
    );
  };

  return <div className="media-panel">
    <Segmented className="media-filter" count={FILTERS.length} index={filter} role="group" aria-label="媒体类型">
      {FILTERS.map((f, i) => <button type="button" key={f.label} aria-pressed={filter === i} onClick={() => setFilter(i)}>{f.label}</button>)}
    </Segmented>
    {state.status === 'loading' && <p className="empty-text" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在读取</p>}
    {state.status === 'error' && <p className="empty-text" role="alert">{state.message} <button type="button" className="text-button" onClick={() => setRound(n => n + 1)}>重试</button></p>}
    {state.status === 'ready' && (state.items.length
      ? <div className="media-list"><HoverTrack items=".media-item" />
        {state.items.map(item => <MediaItem key={keyOf(item)} sessionId={sessionId} item={item} onOpen={onOpen} />)}
        {state.hasMore && <button type="button" className="secondary-button media-more" onClick={more} disabled={state.more}>{state.more ? <><CircleNotch size={14} className="spin" />正在读取</> : '加载更多'}</button>}
        {state.error && <p className="empty-text" role="alert">{state.error}</p>}
      </div>
      : <p className="empty-text">{filter ? '这个会话没有这类媒体。' : '这个会话没有图片、视频或文件。'}</p>)}
  </div>;
}

function MediaItem({ sessionId, item, onOpen }: { sessionId: string; item: Resource; onOpen: (item: Resource) => void }) {
  const [broken, setBroken] = useState(false);
  const label = item.resourceType === 'file' ? item.fileName || '文件' : item.resourceType === 'video' ? '视频' : '图片';
  const detail = [day(item.createTime) + ' ' + formatClock(item.createTime), item.resourceType === 'file' ? size(item.fileSize) : ''].filter(Boolean).join(' · ');
  const thumb = item.resourceType === 'image' && !broken
    ? <img src={mediaUrl('image-data', sessionId, item as unknown as ParsedMessage, { thumb: true })} alt="" loading="lazy" decoding="async" onError={() => setBroken(true)} />
    : <span className="media-icon" aria-hidden="true">{item.resourceType === 'video' ? <VideoCamera size={24} /> : <FileText size={24} />}</span>;
  return <button className="media-item" type="button" aria-label={label + '，' + detail + '，在会话中定位'} onClick={() => onOpen(item)}>
    {thumb}<span><span className="clamp-1">{label}</span><small>{detail}</small></span><ArrowSquareOut size={14} />
  </button>;
}
