// The workspace from the #130 prototype: navigation on the left, records in the middle, the operations
// console on the right as in Claude.ai (#130).
// Records are still sample data (src/mock); the connection state is the real `weflow serve`.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import {
  Aperture, ArrowClockwise, ArrowLeft, ArrowSquareOut, CalendarBlank,
  ChatCircleDots, Check, CheckCircle, Circle, CircleNotch,
  DownloadSimple, FilePdf, Gear, MagnifyingGlass, Plugs, SidebarSimple, Wallet, X,
} from '@phosphor-icons/react';
import { useBackend, WEB, type Backend } from './api/backend';
import { useScope } from './api/scope';
import type { ChatSession } from './api/chat';
import { Avatar } from './Avatar';
import { DateRange } from './calendar/DateRange';
import { CardStack, type Card } from './CardStack';
import { SessionList } from './chat/SessionList';
import { formatSessionTime, kindLabels, kindOf } from './chat/sessions';
import { useSessions, type SessionsState } from './chat/useSessions';
import { LiveMessages } from './chat/LiveMessages';
import { SearchPanel, searchSummary } from './chat/SearchPanel';
import type { Jump } from './chat/useMessages';
import { useSearch } from './chat/useSearch';
import { parseDay } from './calendar/dates';
import type { SearchHit } from './api/messages';
import { PreviewImage, PreviewVideo, type Preview } from './chat/MessageView';
import { exportConversation } from './export';
import { categoryLabels, conversations, CURRENT_DATE, DEFAULT_FROM, formatDay, sampleSessions } from './mock/data';
import { FEEDBACK_MS, usePresence } from './motion';
import { HoverTrack } from './HoverTrack';
import { Scrollbar } from './Scrollbar';
import { Segmented } from './Segmented';
import type { Category, Conversation, ExportFormat, ExportResult, Navigation } from './types';

type ExportStage = 'idle' | 'exporting' | 'done' | 'error';
// Previews of the sample records and of real messages (images and videos of #198).

const navigation = [
  { id: 'chat', label: '聊天', icon: ChatCircleDots },
  { id: 'moments', label: '朋友圈', icon: Aperture },
  { id: 'payments', label: '微信支付', icon: Wallet },
  { id: 'settings', label: '设置', icon: Gear },
] as const;

const formats = ['HTML', 'JSON', 'Excel'] as const;

const backendLabels: Record<Backend['state'], string> = {
  checking: '正在连接', ready: '已连接', offline: '未连接', incompatible: '版本不兼容',
};

function backendDetail(backend: Backend): string {
  switch (backend.state) {
    case 'checking': return '正在检查 weflow serve。';
    case 'offline': return '没有找到 weflow serve，请先运行 weflow serve。';
    case 'incompatible': return backend.reason + '。';
    case 'ready': return 'weflow ' + backend.health.version + ' · API ' + backend.health.apiVersion;
  }
}

/** How long a connection stays 「已连接」 before its notice fades away. */
const STEADY_MS = 3000;
const SIDEBAR_KEY = 'weflow.sidebar';
const LIVE_KEY = 'weflow.live';

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => Boolean(window.matchMedia?.(query).matches));
  useEffect(() => {
    const list = window.matchMedia?.(query);
    if (!list) return;
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener('change', update);
    return () => list.removeEventListener('change', update);
  }, [query]);
  return matches;
}

/** True once the backend has stayed ready for STEADY_MS; any other state shows the notice again at once. */
function useSteadyConnection(state: Backend['state']): boolean {
  const [steady, setSteady] = useState(false);
  useEffect(() => {
    setSteady(false);
    if (state !== 'ready') return;
    const timer = window.setTimeout(() => setSteady(true), STEADY_MS);
    return () => window.clearTimeout(timer);
  }, [state]);
  return steady && state === 'ready';
}

function prefersReducedMotion(setting: boolean): boolean {
  return setting || Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

const SAMPLE_SESSIONS: SessionsState = { status: 'ready', sessions: sampleSessions, statuses: {} };
// Until /health has answered, neither the sample nor the real list: no sample flashes up before the real one.
const CHECKING: SessionsState = { status: 'loading' };

/** A conversation of `weflow serve`, for the console; its messages are read by #198. */
function liveConversation(session: ChatSession | undefined): Conversation {
  if (!session) return { id: '', category: 'chat', name: '', preview: '', stamp: '', description: '', messages: [], live: true };
  const count = session.messageCountHint;
  return {
    id: session.username, category: 'chat', name: session.displayName || session.username, avatar: session.avatarUrl,
    preview: session.summary, stamp: formatSessionTime(session.lastTimestamp),
    description: kindLabels[kindOf(session.username)] + (count ? ' · 约 ' + count + ' 条记录' : ''),
    messages: [], live: true, messageCount: count, lastTime: session.lastTimestamp,
  };
}

/** The export button; while exporting it grows a cancel button, when done it offers the file and another run. */
function ExportControl({ stage, result, onStart, onCancel }: {
  stage: ExportStage; result: ExportResult | null; onStart: () => void; onCancel: () => void;
}) {
  const expanded = stage === 'exporting' || (stage === 'done' && result !== null);
  return <div className={'export-control' + (expanded ? ' expanded' : '')}>
    {stage === 'done' && result
      ? <a className="primary-button large" href={result.downloadUrl} download={result.fileName}><span className="morph-label" key="done"><DownloadSimple size={18} />下载文件</span></a>
      : <button type="button" className="primary-button large" aria-disabled={stage === 'exporting'} onClick={() => { if (stage !== 'exporting') onStart(); }}>
        <span className="morph-label" key={stage}>{stage === 'exporting'
          ? <><CircleNotch size={18} className="spin" />正在导出</>
          : <><DownloadSimple size={18} />{stage === 'error' ? '重新导出' : '开始导出'}</>}</span>
      </button>}
    <button type="button" className="export-side" inert={!expanded} aria-label={stage === 'done' ? '再次导出' : '取消导出'} title={stage === 'done' ? '再次导出' : '取消导出'} onClick={stage === 'done' ? onStart : onCancel}>
      {stage === 'done' ? <ArrowClockwise size={18} /> : <X size={18} />}
    </button>
  </div>;
}

const exportStatus: Record<ExportStage, string> = { idle: '等待导出', exporting: '正在导出', done: '已完成', error: '未完成' };

function TaskStatus({ conversation, from, to, connected, stage, result, error }: {
  conversation: Conversation; from: string; to: string; connected: boolean;
  stage: ExportStage; result: ExportResult | null; error: string;
}) {
  const completed = stage === 'done';
  return <ol className="task-list">
      <li className={connected ? 'task complete' : 'task'}>
        <span className="task-icon">{connected ? <Check size={12} weight="bold" /> : <Circle size={16} />}</span>
        <div><strong>{connected ? '已连接 weflow serve' : '未连接 weflow serve'}</strong></div><time>14:10</time>
      </li>
      <li className="task complete">
        <span className="task-icon"><Check size={12} weight="bold" /></span>
        <div>{conversation.live
          ? <strong>{conversation.name ? '已选择' + conversation.name : '还没有选择会话'}</strong>
          : <><strong>已载入{conversation.name}的{conversation.category === 'chat' ? '聊天记录' : '记录'}</strong><small>({from || '不限'} ~ {to || '不限'})</small></>}</div><time>14:11</time>
      </li>
      <li className={'task last ' + (stage === 'error' ? 'error' : completed ? 'complete' : '')}>
        <span className="task-icon">{stage === 'exporting' ? <CircleNotch size={16} className="spin" /> : completed ? <Check size={12} weight="bold" /> : <Circle size={16} />}</span>
        <div aria-live="polite"><strong>{stage === 'exporting' ? '正在生成导出文件' : completed ? '导出完成' : stage === 'error' ? '导出未完成' : '等待开始导出'}</strong>
          <small>{stage === 'exporting' ? '正在整理记录和媒体文件。' : completed ? '共 ' + String(result?.count || 0) + ' 条记录，文件已生成。' : stage === 'error' ? error : '请确认导出设置并点击开始导出。'}</small>
          {completed && result && <a className="download-link" href={result.downloadUrl} download={result.fileName}>下载文件</a>}
        </div><time>{completed ? '完成' : '--:--'}</time>
      </li>
    </ol>;
}

export function App() {
  // Live messages (#201): on unless turned off, and remembered
  const [realtime, setRealtime] = useState(() => { try { return localStorage.getItem(LIVE_KEY) !== 'off'; } catch { return true; } });
  const toggleRealtime = (on: boolean) => {
    setRealtime(on);
    try { localStorage.setItem(LIVE_KEY, on ? 'on' : 'off'); } catch { /* only lost on reload */ }
  };
  const [backend, recheckBackend] = useBackend(realtime);
  const connected = backend.state === 'ready';
  const steady = useSteadyConnection(backend.state);
  const narrow = useMediaQuery('(max-width: 1100px)');
  const [sidebarFolded, setSidebarFolded] = useState(() => { try { return localStorage.getItem(SIDEBAR_KEY) === 'folded'; } catch { return false; } });
  const folded = narrow || sidebarFolded;
  const toggleSidebar = () => {
    const next = !sidebarFolded;
    setSidebarFolded(next);
    try { localStorage.setItem(SIDEBAR_KEY, next ? 'folded' : 'open'); } catch { /* only lost on reload */ }
  };
  const [section, setSection] = useState<Navigation>('chat');
  const [activeCategory, setActiveCategory] = useState<Category>('chat');
  const [selectedIds, setSelectedIds] = useState<Record<Category, string>>({ chat: 'lin', moments: 'moment-lin', payments: 'payment-coffee' });
  const [from, setFrom] = useState(DEFAULT_FROM);
  const [to, setTo] = useState(CURRENT_DATE);
  const [format, setFormat] = useState<ExportFormat>('HTML');
  const [includeMedia, setIncludeMedia] = useState(true);
  const [stage, setStage] = useState<ExportStage>('idle');
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [error, setError] = useState('');
  const [query, setQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState<string | null>(null);
  const [searchDatesOpen, setSearchDatesOpen] = useState(false);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [toast, setToast] = useState('');
  const [mobileDetail, setMobileDetail] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const consoleRef = useRef<HTMLElement>(null);
  const settingsRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  const exportAbort = useRef<AbortController | null>(null);
  const shownPreview = usePresence(preview);
  const shownToast = usePresence(toast);

  // The chat page reads `weflow serve` once it is connected; until then, and on the other pages, sample records.
  const live = activeCategory === 'chat' && (connected || backend.state === 'checking');
  // another server or account (#198): what was read for the one before goes
  const scope = useScope(backend);
  const [liveSessions, reloadSessions] = useSessions(connected, scope);
  const [liveId, setLiveId] = useState<string | null>(null);
  const sessionList = liveSessions.status === 'ready' ? liveSessions.sessions : [];
  const liveSession = sessionList.find(session => session.username === liveId);
  const liveSearch = useSearch();
  // A day picked or a search hit opened: its conversation shows the messages there (#199).
  const [jumpTo, setJumpTo] = useState<Jump | null>(null);
  const jumps = useRef(0);
  const { clear: clearSearch } = liveSearch;
  useEffect(() => {
    if (!scope) return;
    setJumpTo(null);
    clearSearch();
  }, [scope, clearSearch]);
  useEffect(() => {
    if (!liveSession && sessionList.length) setLiveId(sessionList[0].username);
  }, [liveSession, sessionList]);
  const list = conversations.filter(item => item.category === activeCategory);
  const selected = live ? liveConversation(liveSession) : list.find(item => item.id === selectedIds[activeCategory]) || list[0];
  const media = selected.messages.filter(message => message.kind !== 'text');
  const results = useMemo(() => submittedQuery === null ? [] : selected.messages.filter(message => {
    const matchesDate = !searchDatesOpen || ((!from || message.date >= from) && (!to || message.date <= to));
    return matchesDate && (message.text + ' ' + message.sender).toLocaleLowerCase().includes(submittedQuery.toLocaleLowerCase());
  }), [selected, submittedQuery, searchDatesOpen, from, to]);

  useEffect(() => {
    document.documentElement.classList.toggle('reduced-motion', reducedMotion);
  }, [reducedMotion]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(''), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => () => {
    if (exportResult) URL.revokeObjectURL(exportResult.downloadUrl);
  }, [exportResult]);

  useEffect(() => {
    if (!highlighted) return;
    document.getElementById('message-' + highlighted)?.scrollIntoView({ block: 'center', behavior: prefersReducedMotion(reducedMotion) ? 'auto' : 'smooth' });
    const timer = window.setTimeout(() => setHighlighted(null), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [highlighted, reducedMotion]);

  // The dialog stays in the top layer until its exit animation has played.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (shownPreview && dialog && !dialog.open) dialog.showModal?.();
    else if (!shownPreview && dialog?.open) dialog.close();
  }, [shownPreview]);

  const resetExport = () => {
    exportAbort.current?.abort();
    exportAbort.current = null;
    setStage('idle'); setError(''); setExportResult(null);
  };

  const selectSection = (value: Navigation) => {
    setSection(value); setMobileDetail(false);
    if (value !== 'settings') { setActiveCategory(value); resetExport(); setSubmittedQuery(null); setQuery(''); }
  };

  const selectConversation = (id: string) => {
    if (live) {
      setLiveId(id);
      // a search of the conversation left behind is over; one of all conversations still holds
      if (liveSearch.state.query?.sessionId) liveSearch.clear();
    }
    else setSelectedIds(current => ({ ...current, [activeCategory]: id }));
    setMobileDetail(true); setSubmittedQuery(null); setQuery(''); resetExport();
  };

  // On a phone the console is below the messages: after a jump, the page goes up to them.
  const showMessages = () => {
    setMobileDetail(true);
    if (!window.matchMedia?.('(max-width: 640px)').matches) return;
    requestAnimationFrame(() => paneRef.current?.closest('.browser-panel')?.scrollIntoView({ block: 'start', behavior: prefersReducedMotion(reducedMotion) ? 'auto' : 'smooth' }));
  };

  const openHit = (hit: SearchHit) => {
    if (!sessionList.some(session => session.username === hit._session_id)) { setToast('这条消息所在的会话不在会话列表里。'); return; }
    setLiveId(hit._session_id);
    setJumpTo({ sessionId: hit._session_id, target: { at: 'message', message: hit }, seq: ++jumps.current });
    showMessages();
  };

  const jumpToDay = (sessionId: string, day: string) => {
    setJumpTo({ sessionId, target: { at: 'time', seconds: parseDay(day).getTime() / 1000 }, seq: ++jumps.current });
    showMessages();
  };

  const exportRecords = async () => {
    exportAbort.current?.abort();
    const controller = new AbortController();
    exportAbort.current = controller;
    setStage('exporting'); setError(''); setExportResult(null);
    try {
      const result = await exportConversation(selected, { format, from, to, includeMedia }, controller.signal);
      if (exportAbort.current === controller) { setExportResult(result); setStage('done'); }
      else URL.revokeObjectURL(result.downloadUrl);
    } catch (problem) {
      if (exportAbort.current === controller) { setError(problem instanceof Error ? problem.message : '导出失败，请重试。'); setStage('error'); }
    }
  };

  const search = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSubmittedQuery(query.trim());
  };

  const cards: Card[] = [
    {
      id: 'export', title: '导出', summary: format + (includeMedia ? ' · 含媒体' : ''),
      content: <>
        <p className="card-lead"><span className="clamp-1">{selected.name}的{selected.category === 'chat' ? '聊天记录' : '记录'}</span>{!selected.live && <span className="badge">示例数据</span>}</p>
        <div className="form-row"><span className="field-label">时间范围</span><DateRange label="导出的时间范围" from={from} to={to} onChange={range => { setFrom(range.from); setTo(range.to); resetExport(); }} /></div>
        <div className="form-row format-row"><span className="field-label">导出格式</span>
          <Segmented className="format-group" count={formats.length} index={formats.indexOf(format)} role="group" aria-label="导出格式">
            {formats.map(value => <button type="button" key={value} aria-pressed={format === value} onClick={() => { setFormat(value); resetExport(); }}>{value}</button>)}
          </Segmented>
          <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={includeMedia} onChange={event => { setIncludeMedia(event.target.checked); resetExport(); }} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>附带媒体文件 <small>图片、语音、视频与表情</small></span></label>
        </div>
        {selected.live
          ? <><button type="button" className="primary-button large export-pending" disabled><DownloadSimple size={18} />开始导出</button><p className="card-note">真实记录的导出将由 weflow serve 生成，在导出页（#211）接入。</p></>
          : <ExportControl stage={stage} result={exportResult} onStart={exportRecords} onCancel={resetExport} />}
      </>,
    },
    {
      id: 'tasks', title: '当前任务',
      summary: <><span className={'status-dot ' + stage} />{exportStatus[stage]}</>,
      content: <TaskStatus conversation={selected} from={from} to={to} connected={connected} stage={stage} result={exportResult} error={error} />,
    },
    {
      id: 'search', title: '搜索', summary: selected.live ? searchSummary(liveSearch.state) : submittedQuery === null ? '当前会话' : results.length + ' 条结果',
      content: selected.live
        ? <SearchPanel key={scope} session={liveSession} sessions={sessionList} search={liveSearch} onOpenHit={openHit} onJumpDay={jumpToDay} />
        : <>
        <form className="search-form" onSubmit={search}>
          <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" /><input type="search" aria-label="搜索此会话的消息" placeholder={'搜索' + selected.name + '的消息'} value={query} onChange={event => setQuery(event.target.value)} /><button className={'icon-button small' + (searchDatesOpen ? ' on' : '')} type="button" aria-label="筛选搜索日期" aria-expanded={searchDatesOpen} onClick={() => setSearchDatesOpen(value => !value)}><CalendarBlank size={16} /></button></div>
          <button className="primary-button search-button" type="submit" aria-label="搜索"><MagnifyingGlass size={16} /><span className="fold-label">搜索</span></button>
        </form>
        {searchDatesOpen && <div className="search-date-row"><span className="field-label">时间范围</span><DateRange label="搜索的时间范围" from={from} to={to} onChange={range => { setFrom(range.from); setTo(range.to); }} /></div>}
        <div className="search-results" aria-live="polite"><HoverTrack items=".search-result" />{submittedQuery === null
          ? <p className="empty-text">输入关键词，搜索当前会话的记录。</p>
          : results.length
            ? <><p className="results-count">找到 {results.length} 条记录<span>点击可定位到消息</span></p>{results.map(message => <button className="search-result" type="button" key={message.id} onClick={() => setHighlighted(message.id)}><span className="search-result-copy"><strong>{message.sender}</strong><span className="clamp-1">{message.text}</span></span><time>{message.date} {message.time}</time><ArrowSquareOut size={14} /></button>)}</>
            : <p className="empty-text">没有找到匹配的消息，试试其他关键词或时间范围。</p>}</div>
      </>,
    },
    {
      id: 'info', title: '会话资料', summary: selected.live ? (selected.messageCount ? '约 ' + selected.messageCount + ' 条' : '') : selected.messages.length + ' 条记录',
      content: <>
        <div className="info-content"><Avatar name={selected.name} url={selected.avatar} /><div><h4>{selected.name}</h4><p className="clamp-2">{selected.description}</p></div></div>
        {selected.live
          ? <dl className="info-grid"><div><dt>类型</dt><dd>{kindLabels[kindOf(selected.id)]}</dd></div><div><dt>记录条数</dt><dd className="num">{selected.messageCount ? '约 ' + selected.messageCount + ' 条' : '—'}</dd></div><div><dt>最后一条消息</dt><dd className="num">{selected.lastTime ? new Date(selected.lastTime * 1000).toLocaleString('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }) : '—'}</dd></div></dl>
          : <dl className="info-grid"><div><dt>类型</dt><dd>{categoryLabels[selected.category]}</dd></div><div><dt>记录条数</dt><dd className="num">{selected.messages.length} 条</dd></div><div><dt>媒体文件</dt><dd className="num">{media.length} 个</dd></div><div><dt>时间范围</dt><dd className="num">{selected.messages[0]?.date} ↔ {selected.messages.at(-1)?.date}</dd></div></dl>}
      </>,
    },
    {
      id: 'media', title: '媒体文件', summary: media.length + ' 个',
      content: <div className="media-list"><HoverTrack items=".media-item" />{media.length ? media.map(message => <button className="media-item" type="button" key={message.id} onClick={() => message.asset ? setPreview({ type: 'image', src: message.asset, title: message.text }) : setPreview({ type: 'file', title: message.fileName || message.text })}>{message.asset ? <img src={message.asset} alt="" /> : <FilePdf size={32} />}<span><span className="clamp-1">{message.fileName || message.text}</span><small>{message.date} {message.time}</small></span><ArrowSquareOut size={14} /></button>) : <p className="empty-text">这段记录中没有媒体文件。</p>}</div>,
    },
  ];
  // The media of a real conversation come with #200.
  const hiddenCards = selected.live ? ['media'] : [];

  return <main className={'workspace' + (folded ? ' sidebar-collapsed' : '')}>
    <aside className="sidebar">
      <nav aria-label="主导航"><HoverTrack items=".nav-item" />
        {navigation.map(item => {
          const Icon = item.icon;
          const current = section === item.id;
          return <button key={item.id} type="button" className={'nav-item' + (current ? ' selected' : '')} aria-current={current ? 'page' : undefined} aria-label={item.label} title={item.label} onClick={() => selectSection(item.id)}>
            <Icon size={18} weight={current ? 'fill' : 'regular'} aria-hidden="true" /><span className="nav-label">{item.label}</span>
          </button>;
        })}
      </nav>
      <div className="sidebar-footer">
        {!narrow && <button type="button" className="icon-button sidebar-toggle" aria-label={folded ? '展开侧边栏' : '收起侧边栏'} title={folded ? '展开侧边栏' : '收起侧边栏'} aria-expanded={!folded} onClick={toggleSidebar}><SidebarSimple size={18} /></button>}
        <div className={'connection ' + backend.state + (steady ? ' quiet' : '')} role="status" title={backendDetail(backend)}><span className="connection-dot" /><span className="connection-label">{connected && realtime ? '实时' : backendLabels[backend.state]}</span></div>
      </div>
    </aside>

    <div className="main-area">
      {section === 'settings' ? <><section ref={settingsRef} className="panel browser-panel settings-panel scroll-area" aria-label="设置">
        <div className="settings-heading"><Gear size={20} /><h1>设置</h1></div>
        <p className="muted">管理连接和浏览偏好。</p>
        <div className="setting-row"><div><h2>weflow serve</h2><p>{backendDetail(backend)}</p></div><button className="secondary-button" type="button" onClick={recheckBackend}>重新检测</button></div>
        <div className="setting-row"><div><h2>网页版本</h2><p>需要 API {WEB.apiVersion}，weflow {WEB.minBackendVersion} 或更新的版本。</p></div><span className="source-name"><Plugs size={14} />{WEB.version}</span></div>
        <div className="setting-row"><div><h2>实时消息</h2><p>微信收到消息后几秒内显示在会话列表和打开的会话里。关掉后不再接收，左下角显示「已连接」而不是「实时」。</p></div><label className="toggle"><input type="checkbox" aria-label="实时消息" checked={realtime} onChange={event => toggleRealtime(event.target.checked)} /><span /></label></div>
        <div className="setting-row"><div><h2>默认附带媒体文件</h2><p>导出时同时保存照片等媒体。</p></div><label className="toggle"><input type="checkbox" aria-label="默认附带媒体文件" checked={includeMedia} onChange={event => setIncludeMedia(event.target.checked)} /><span /></label></div>
        <div className="setting-row"><div><h2>减少动画</h2><p>所有过渡立即完成，状态提示保留。也跟随系统的「减少动态效果」。</p></div><label className="toggle"><input type="checkbox" aria-label="减少动画" checked={reducedMotion} onChange={event => setReducedMotion(event.target.checked)} /><span /></label></div>
        <small className="settings-footnote">聊天记录目前是示例数据，页面接入 API 后替换。</small>
      </section><Scrollbar target={settingsRef} /></> : <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label={categoryLabels[activeCategory] + '记录浏览'}>
        {activeCategory === 'chat'
          ? <SessionList state={connected ? liveSessions : backend.state === 'checking' ? CHECKING : SAMPLE_SESSIONS} sample={!connected && backend.state !== 'checking'} selectedId={selected.id || null} onSelect={selectConversation} onRetry={reloadSessions} />
          : <><div ref={listRef} className="conversation-list scroll-area" aria-label={categoryLabels[activeCategory] + '列表'}><HoverTrack items=".conversation-row" />
          {list.map(conversation => <button type="button" key={conversation.id} className={'conversation-row' + (conversation.id === selected.id ? ' active' : '')} aria-label={'选择' + conversation.name} aria-pressed={conversation.id === selected.id} onClick={() => selectConversation(conversation.id)}>
            <Avatar name={conversation.name} url={conversation.avatar} />
            <span className="conversation-copy">
              <span className="conversation-head"><span className="conversation-name">{conversation.name}</span><time className="conversation-time">{conversation.stamp}</time></span>
              <span className="conversation-preview clamp-1">{conversation.preview}</span>
            </span>
          </button>)}
        </div><Scrollbar target={listRef} /></>}
        <div ref={paneRef} className="message-pane scroll-area" aria-label={selected.name + '的记录'}>
          <button type="button" className="mobile-back icon-button" aria-label="返回对话列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
          {selected.live && (liveSession
            ? <LiveMessages key={scope} session={liveSession} scroller={paneRef} jump={jumpTo} onPreview={setPreview} />
            : <div className="pane-message">{liveSessions.status === 'ready' && <p>选择左侧的一个会话。</p>}</div>)}
          {!selected.live && <div className="messages">
            {selected.messages.map((message, index) => <div key={message.id}>
              {(index === 0 || selected.messages[index - 1].date !== message.date) && <div className="date-separator">{formatDay(message.date)}</div>}
              <div id={'message-' + message.id} className={'message-row ' + message.direction + (highlighted === message.id ? ' highlighted' : '')}>
                {message.kind === 'image' && message.asset
                  ? <button type="button" className="photo-button" aria-label={'预览' + message.text} onClick={() => setPreview({ type: 'image', src: message.asset!, title: message.text })}><img className="message-photo" src={message.asset} alt={message.text} /></button>
                  : message.kind === 'file'
                    ? <button className="message-bubble file-bubble" type="button" onClick={() => setPreview({ type: 'file', title: message.fileName || message.text })}><FilePdf size={28} /><span>{message.fileName}<small>示例文件</small></span></button>
                    : <div className="message-bubble">{message.text}</div>}
                <time className="message-time">{message.time}</time>
              </div>
            </div>)}
          </div>}
        </div>
        <Scrollbar target={paneRef} />
      </section>}

      <section ref={consoleRef} className="console scroll-area" aria-label="操作控制台">
        <CardStack cards={cards} hidden={hiddenCards} storageKey="weflow.console" defaultClosed={['search', 'info', 'media']} label="操作控制台" />
      </section>
      <Scrollbar target={consoleRef} />
    </div>

    <dialog className="preview-dialog" ref={dialogRef} aria-label={shownPreview?.value.title || '媒体预览'} data-state={shownPreview?.closing ? 'closing' : 'open'}
      onCancel={event => { event.preventDefault(); setPreview(null); }} onClick={event => { if (event.target === event.currentTarget) setPreview(null); }}>
      {shownPreview && <div className="preview-content"><header><strong>{shownPreview.value.title}</strong><button type="button" className="icon-button" aria-label="关闭预览" onClick={() => setPreview(null)}><X size={18} /></button></header>
        {shownPreview.value.type === 'image'
          ? <PreviewImage src={shownPreview.value.src} title={shownPreview.value.title} />
          : shownPreview.value.type === 'video'
            ? <PreviewVideo src={shownPreview.value.src} poster={shownPreview.value.poster} />
            : <div className="file-preview"><FilePdf size={56} /><h3>{shownPreview.value.title}</h3><p>示例文件记录，可查看名称与记录时间。</p></div>}</div>}
    </dialog>
    {shownToast && <div className="toast" role="status" data-state={shownToast.closing ? 'closing' : 'open'}><CheckCircle size={16} />{shownToast.value}</div>}
  </main>;
}
