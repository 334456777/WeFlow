// The workspace from the #130 prototype: navigation on the left, records in the middle, the operations
// console on the right as in Claude.ai (#130).
// Records are still sample data (src/mock); the connection state is the real `weflow serve`.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, FormEvent, KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import {
  Aperture, ArrowClockwise, ArrowLeft, ArrowsLeftRight, ArrowSquareOut, Article, CalendarBlank,
  ChatCircleDots, Check, CheckCircle, CaretDown, Circle, CircleNotch,
  DownloadSimple, FilePdf, Gear, ImageSquare, ListBullets,
  MagnifyingGlass, Plugs, SlidersHorizontal, Wallet, X,
} from '@phosphor-icons/react';
import { useBackend, WEB, type Backend } from './api/backend';
import { exportConversation } from './export';
import { categoryLabels, conversations, CURRENT_DATE, DEFAULT_FROM, formatDay } from './mock/data';
import { FEEDBACK_MS, usePresence } from './motion';
import type { Category, ConsoleMode, Conversation, ExportFormat, ExportResult, Navigation } from './types';

type ExportStage = 'idle' | 'exporting' | 'done' | 'error';
type Preview = { type: 'image'; src: string; title: string } | { type: 'file'; title: string };

const navigation = [
  { id: 'chat', label: '聊天', icon: ChatCircleDots },
  { id: 'moments', label: '朋友圈', icon: Aperture },
  { id: 'articles', label: '公众号', icon: Article },
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

function prefersReducedMotion(setting: boolean): boolean {
  return setting || Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

function Avatar({ conversation }: { conversation: Conversation }) {
  return conversation.avatar
    ? <img className="avatar" src={conversation.avatar} alt="" />
    : <span className="avatar avatar-initial" aria-hidden="true">{Array.from(conversation.name)[0]}</span>;
}

/** Options in a pill with a thumb that slides to the selected one. */
function Segmented({ count, index, className, children, ...rest }: {
  count: number; index: number; className: string; children: ReactNode;
  role?: string; 'aria-label': string; onKeyDown?: (event: ReactKeyboardEvent<HTMLDivElement>) => void;
}) {
  const style = { '--count': count, '--index': index } as CSSProperties;
  return <div className={'segmented ' + className} style={style} {...rest}>
    <span className="segmented-thumb" aria-hidden="true" />{children}
  </div>;
}

function DateRange({ from, to, onFrom, onTo }: { from: string; to: string; onFrom: (value: string) => void; onTo: (value: string) => void }) {
  return <div className="date-range">
    <label className="date-field"><CalendarBlank size={16} aria-hidden="true" /><span className="date-display" aria-hidden="true">{from || '选择日期'}</span>
      <input type="date" aria-label="开始日期" value={from} onClick={event => event.currentTarget.showPicker?.()} onInput={event => onFrom(event.currentTarget.value)} onChange={event => onFrom(event.target.value)} onBlur={event => { if (event.currentTarget.value !== from) onFrom(event.currentTarget.value); }} />
    </label>
    <ArrowsLeftRight className="range-arrow" size={16} aria-hidden="true" />
    <label className="date-field"><CalendarBlank size={16} aria-hidden="true" /><span className="date-display" aria-hidden="true">{to || '选择日期'}</span>
      <input type="date" aria-label="结束日期" value={to} onClick={event => event.currentTarget.showPicker?.()} onInput={event => onTo(event.currentTarget.value)} onChange={event => onTo(event.target.value)} onBlur={event => { if (event.currentTarget.value !== to) onTo(event.currentTarget.value); }} />
    </label>
  </div>;
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

function TaskStatus({ conversation, from, to, connected, stage, result, error }: {
  conversation: Conversation; from: string; to: string; connected: boolean;
  stage: ExportStage; result: ExportResult | null; error: string;
}) {
  const completed = stage === 'done';
  return <aside className="task-area" aria-label="当前任务">
    <h3><ListBullets size={16} aria-hidden="true" />当前任务</h3>
    <ol className="task-list">
      <li className={connected ? 'task complete' : 'task'}>
        <span className="task-icon">{connected ? <Check size={12} weight="bold" /> : <Circle size={16} />}</span>
        <div><strong>{connected ? '已连接 weflow serve' : '未连接 weflow serve'}</strong></div><time>14:10</time>
      </li>
      <li className="task complete">
        <span className="task-icon"><Check size={12} weight="bold" /></span>
        <div><strong>已载入{conversation.name}的{conversation.category === 'chat' ? '聊天记录' : '记录'}</strong><small>({from || '不限'} ~ {to || '不限'})</small></div><time>14:11</time>
      </li>
      <li className={'task last ' + (stage === 'error' ? 'error' : completed ? 'complete' : '')}>
        <span className="task-icon">{stage === 'exporting' ? <CircleNotch size={16} className="spin" /> : completed ? <Check size={12} weight="bold" /> : <Circle size={16} />}</span>
        <div aria-live="polite"><strong>{stage === 'exporting' ? '正在生成导出文件' : completed ? '导出完成' : stage === 'error' ? '导出未完成' : '等待开始导出'}</strong>
          <small>{stage === 'exporting' ? '正在整理记录和媒体文件。' : completed ? '共 ' + String(result?.count || 0) + ' 条记录，文件已生成。' : stage === 'error' ? error : '请确认导出设置并点击开始导出。'}</small>
          {completed && result && <a className="download-link" href={result.downloadUrl} download={result.fileName}>下载文件</a>}
        </div><time>{completed ? '完成' : '--:--'}</time>
      </li>
    </ol>
  </aside>;
}

export function App() {
  const [backend, recheckBackend] = useBackend();
  const connected = backend.state === 'ready';
  const [section, setSection] = useState<Navigation>('chat');
  const [activeCategory, setActiveCategory] = useState<Category>('chat');
  const [selectedIds, setSelectedIds] = useState<Record<Category, string>>({ chat: 'lin', moments: 'moment-lin', articles: 'article-design', payments: 'payment-coffee' });
  const [mode, setMode] = useState<ConsoleMode>('export');
  const [moreOpen, setMoreOpen] = useState(false);
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
  const moreRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const exportAbort = useRef<AbortController | null>(null);
  const menu = usePresence(moreOpen);
  const shownPreview = usePresence(preview);
  const shownToast = usePresence(toast);

  const list = conversations.filter(item => item.category === activeCategory);
  const selected = list.find(item => item.id === selectedIds[activeCategory]) || list[0];
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

  useEffect(() => {
    const close = (event: MouseEvent) => { if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false); };
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (moreRef.current?.contains(document.activeElement)) moreRef.current.querySelector('button')?.focus();
      setMoreOpen(false);
    };
    document.addEventListener('click', close);
    document.addEventListener('keydown', key);
    return () => { document.removeEventListener('click', close); document.removeEventListener('keydown', key); };
  }, []);

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

  const selectConversation = (conversation: Conversation) => {
    setSelectedIds(current => ({ ...current, [activeCategory]: conversation.id }));
    setMobileDetail(true); setSubmittedQuery(null); setQuery(''); resetExport();
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
    if (searchDatesOpen && from && to && from > to) { setToast('开始日期不能晚于结束日期。'); return; }
    setSubmittedQuery(query.trim());
  };

  const moveConsoleFocus = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const isMenu = (event.target as HTMLElement).getAttribute('role') === 'menuitem';
    const forward = isMenu ? 'ArrowDown' : 'ArrowRight';
    const backward = isMenu ? 'ArrowUp' : 'ArrowLeft';
    if (![forward, backward, 'Home', 'End'].includes(event.key)) return;
    const controls = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(isMenu ? 'button[role="menuitem"]' : 'button[role="tab"]'));
    const index = controls.indexOf(document.activeElement as HTMLButtonElement);
    if (index < 0) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? controls.length - 1 : (index + (event.key === forward ? 1 : -1) + controls.length) % controls.length;
    controls[next]?.focus();
  };

  return <main className="workspace">
    <aside className="sidebar">
      <nav aria-label="主导航">
        {navigation.map(item => {
          const Icon = item.icon;
          const current = section === item.id;
          return <button key={item.id} type="button" className={'nav-item' + (current ? ' selected' : '')} aria-current={current ? 'page' : undefined} aria-label={item.label} title={item.label} onClick={() => selectSection(item.id)}>
            <Icon size={18} weight={current ? 'fill' : 'regular'} aria-hidden="true" /><span className="nav-label">{item.label}</span>
          </button>;
        })}
      </nav>
      <div className={'connection ' + backend.state} role="status" title={backendDetail(backend)}><span className="connection-dot" /><span className="connection-label">{backendLabels[backend.state]}</span></div>
    </aside>

    <div className="main-area">
      {section === 'settings' ? <section className="panel browser-panel settings-panel scroll-area" aria-label="设置">
        <div className="settings-heading"><Gear size={20} /><h1>设置</h1></div>
        <p className="muted">管理连接和浏览偏好。</p>
        <div className="setting-row"><div><h2>weflow serve</h2><p>{backendDetail(backend)}</p></div><button className="secondary-button" type="button" onClick={recheckBackend}>重新检测</button></div>
        <div className="setting-row"><div><h2>网页版本</h2><p>需要 API {WEB.apiVersion}，weflow {WEB.minBackendVersion} 或更新的版本。</p></div><span className="source-name"><Plugs size={14} />{WEB.version}</span></div>
        <div className="setting-row"><div><h2>默认附带媒体文件</h2><p>导出时同时保存照片等媒体。</p></div><label className="toggle"><input type="checkbox" aria-label="默认附带媒体文件" checked={includeMedia} onChange={event => setIncludeMedia(event.target.checked)} /><span /></label></div>
        <div className="setting-row"><div><h2>减少动画</h2><p>所有过渡立即完成，状态提示保留。也跟随系统的「减少动态效果」。</p></div><label className="toggle"><input type="checkbox" aria-label="减少动画" checked={reducedMotion} onChange={event => setReducedMotion(event.target.checked)} /><span /></label></div>
        <small className="settings-footnote">聊天记录目前是示例数据，页面接入 API 后替换。</small>
      </section> : <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label={categoryLabels[activeCategory] + '记录浏览'}>
        <div className="conversation-list scroll-area" aria-label={categoryLabels[activeCategory] + '列表'}>
          {list.map(conversation => <button type="button" key={conversation.id} className={'conversation-row' + (conversation.id === selected.id ? ' active' : '')} aria-label={'选择' + conversation.name} aria-pressed={conversation.id === selected.id} onClick={() => selectConversation(conversation)}>
            <Avatar conversation={conversation} />
            <span className="conversation-copy">
              <span className="conversation-head"><span className="conversation-name">{conversation.name}</span><time className="conversation-time">{conversation.stamp}</time></span>
              <span className="conversation-preview clamp-1">{conversation.preview}</span>
            </span>
          </button>)}
        </div>
        <div className="message-pane scroll-area" aria-label={selected.name + '的记录'}>
          <button type="button" className="mobile-back icon-button" aria-label="返回对话列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
          <div className="messages">
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
          </div>
        </div>
      </section>}

      <section className="panel console-panel" aria-label="操作控制台">
        <header className="console-header"><h2><SlidersHorizontal size={18} aria-hidden="true" />操作控制台</h2>
          <Segmented className="function-group" count={3} index={mode === 'export' ? 0 : mode === 'search' ? 1 : 2} role="tablist" aria-label="控制台功能" onKeyDown={moveConsoleFocus}>
            <button type="button" role="tab" id="export-tab" aria-selected={mode === 'export'} aria-controls="console-content" onClick={() => { setMode('export'); setMoreOpen(false); }}>导出</button>
            <button type="button" role="tab" id="search-tab" aria-selected={mode === 'search'} aria-controls="console-content" onClick={() => { setMode('search'); setMoreOpen(false); }}>搜索</button>
            <div className="more-wrapper" ref={moreRef}><button type="button" role="tab" id="more-tab" aria-selected={mode === 'info' || mode === 'media'} aria-haspopup="menu" aria-expanded={moreOpen} aria-controls="console-content" onClick={() => setMoreOpen(value => !value)}>更多<CaretDown size={14} aria-hidden="true" /></button>
              {menu && <div className="menu" role="menu" aria-label="更多功能" data-state={menu.closing ? 'closing' : 'open'}>
                <button type="button" role="menuitem" className="menu-item" onClick={() => { setMode('info'); setMoreOpen(false); }}><Article size={16} />会话资料</button>
                <button type="button" role="menuitem" className="menu-item" onClick={() => { setMode('media'); setMoreOpen(false); }}><ImageSquare size={16} />媒体文件</button>
              </div>}
            </div>
          </Segmented>
        </header>

        <div id="console-content" role="tabpanel" aria-labelledby={mode === 'export' ? 'export-tab' : mode === 'search' ? 'search-tab' : 'more-tab'} className={'console-body scroll-area ' + mode} key={mode}>
          {mode === 'export' ? <>
            <div className="export-area"><div className="function-title"><h3>导出{selected.name}的{selected.category === 'chat' ? '聊天记录' : '记录'}</h3><span className="badge">示例数据</span></div>
              <div className="form-row"><span className="field-label">时间范围</span><DateRange from={from} to={to} onFrom={value => { setFrom(value); resetExport(); }} onTo={value => { setTo(value); resetExport(); }} /></div>
              <div className="form-row format-row"><span className="field-label">导出格式</span>
                <Segmented className="format-group" count={formats.length} index={formats.indexOf(format)} role="group" aria-label="导出格式">
                  {formats.map(value => <button type="button" key={value} aria-pressed={format === value} onClick={() => { setFormat(value); resetExport(); }}>{value}</button>)}
                </Segmented>
                <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={includeMedia} onChange={event => { setIncludeMedia(event.target.checked); resetExport(); }} /><span>附带媒体文件 <small>图片、语音、视频与表情</small></span></label>
              </div>
              <ExportControl stage={stage} result={exportResult} onStart={exportRecords} onCancel={resetExport} />
            </div>
            <TaskStatus conversation={selected} from={from} to={to} connected={connected} stage={stage} result={exportResult} error={error} />
          </> : mode === 'search' ? <div className="search-area">
            <div className="function-title"><h3>搜索{selected.name}的消息</h3><span className="badge">示例数据</span></div>
            <form className="search-form" onSubmit={search}>
              <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" /><input type="search" aria-label="搜索此会话的消息" placeholder="搜索此会话的消息" value={query} onChange={event => setQuery(event.target.value)} /><button className={'icon-button small' + (searchDatesOpen ? ' on' : '')} type="button" aria-label="筛选搜索日期" aria-expanded={searchDatesOpen} onClick={() => setSearchDatesOpen(value => !value)}><CalendarBlank size={16} /></button></div>
              <button className="primary-button search-button" type="submit" aria-label="搜索"><MagnifyingGlass size={16} /><span className="fold-label">搜索</span></button>
            </form>
            {searchDatesOpen && <div className="search-date-row"><span className="field-label">时间范围</span><DateRange from={from} to={to} onFrom={setFrom} onTo={setTo} /></div>}
            <div className="search-results" aria-live="polite">{submittedQuery === null
              ? <p className="empty-text">输入关键词，搜索当前会话的记录。</p>
              : results.length
                ? <><p className="results-count">找到 {results.length} 条记录<span>点击可定位到消息</span></p>{results.map(message => <button className="search-result" type="button" key={message.id} onClick={() => setHighlighted(message.id)}><span className="search-result-copy"><strong>{message.sender}</strong><span className="clamp-1">{message.text}</span></span><time>{message.date} {message.time}</time><ArrowSquareOut size={14} /></button>)}</>
                : <p className="empty-text">没有找到匹配的消息，试试其他关键词或时间范围。</p>}</div>
          </div> : mode === 'info' ? <div className="info-area">
            <div className="function-title"><h3>会话资料</h3><span className="badge">示例数据</span></div>
            <div className="info-content"><Avatar conversation={selected} /><div><h4>{selected.name}</h4><p className="clamp-2">{selected.description}</p></div></div>
            <dl className="info-grid"><div><dt>类型</dt><dd>{categoryLabels[selected.category]}</dd></div><div><dt>记录条数</dt><dd className="num">{selected.messages.length} 条</dd></div><div><dt>媒体文件</dt><dd className="num">{media.length} 个</dd></div><div><dt>时间范围</dt><dd className="num">{selected.messages[0]?.date} ↔ {selected.messages.at(-1)?.date}</dd></div></dl>
          </div> : <div className="media-area">
            <div className="function-title"><h3>{selected.name}的媒体文件</h3><span className="badge">示例数据</span></div>
            <div className="media-list">{media.length ? media.map(message => <button className="media-item" type="button" key={message.id} onClick={() => message.asset ? setPreview({ type: 'image', src: message.asset, title: message.text }) : setPreview({ type: 'file', title: message.fileName || message.text })}>{message.asset ? <img src={message.asset} alt="" /> : <FilePdf size={32} />}<span><span className="clamp-1">{message.fileName || message.text}</span><small>{message.date} {message.time}</small></span><ArrowSquareOut size={14} /></button>) : <p className="empty-text">这段记录中没有媒体文件。</p>}</div>
          </div>}
        </div>
      </section>
    </div>

    <dialog className="preview-dialog" ref={dialogRef} aria-label={shownPreview?.value.title || '媒体预览'} data-state={shownPreview?.closing ? 'closing' : 'open'}
      onCancel={event => { event.preventDefault(); setPreview(null); }} onClick={event => { if (event.target === event.currentTarget) setPreview(null); }}>
      {shownPreview && <div className="preview-content"><header><strong>{shownPreview.value.title}</strong><button type="button" className="icon-button" aria-label="关闭预览" onClick={() => setPreview(null)}><X size={18} /></button></header>
        {shownPreview.value.type === 'image' ? <img src={shownPreview.value.src} alt={shownPreview.value.title} /> : <div className="file-preview"><FilePdf size={56} /><h3>{shownPreview.value.title}</h3><p>示例文件记录，可查看名称与记录时间。</p></div>}</div>}
    </dialog>
    {shownToast && <div className="toast" role="status" data-state={shownToast.closing ? 'closing' : 'open'}><CheckCircle size={16} />{shownToast.value}</div>}
  </main>;
}
