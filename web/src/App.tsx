// The workspace from the #130 prototype: navigation on the left, records top right, the operations console below.
// Records are still sample data (src/mock); the connection state is the real `weflow serve`.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, KeyboardEvent as ReactKeyboardEvent } from 'react';
import {
  Aperture, ArrowLeft, ArrowsLeftRight, ArrowSquareOut, Article, CalendarBlank,
  ChatCircleDots, Check, CheckCircle, CaretDown, Circle, CircleNotch,
  DownloadSimple, FilePdf, Gear, ImageSquare, ListBullets,
  MagnifyingGlass, Plugs, SlidersHorizontal, Wallet, X,
} from '@phosphor-icons/react';
import { useBackend, WEB, type Backend } from './api/backend';
import { exportConversation } from './export';
import { categoryLabels, conversations, CURRENT_DATE, DEFAULT_FROM, formatDay } from './mock/data';
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

function Avatar({ conversation }: { conversation: Conversation }) {
  return conversation.avatar
    ? <img className="avatar" src={conversation.avatar} alt="" />
    : <span className="avatar avatar-initial" aria-hidden="true">{Array.from(conversation.name)[0]}</span>;
}

function DateRange({ from, to, onFrom, onTo }: { from: string; to: string; onFrom: (value: string) => void; onTo: (value: string) => void }) {
  return <div className="date-range">
    <label className="date-field"><CalendarBlank size={21} aria-hidden="true" /><span className="date-display" aria-hidden="true">{from || '选择日期'}</span>
      <input type="date" aria-label="开始日期" value={from} onClick={event => event.currentTarget.showPicker?.()} onInput={event => onFrom(event.currentTarget.value)} onChange={event => onFrom(event.target.value)} onBlur={event => { if (event.currentTarget.value !== from) onFrom(event.currentTarget.value); }} />
    </label>
    <ArrowsLeftRight className="range-arrow" size={28} aria-hidden="true" />
    <label className="date-field"><CalendarBlank size={21} aria-hidden="true" /><span className="date-display" aria-hidden="true">{to || '选择日期'}</span>
      <input type="date" aria-label="结束日期" value={to} onClick={event => event.currentTarget.showPicker?.()} onInput={event => onTo(event.currentTarget.value)} onChange={event => onTo(event.target.value)} onBlur={event => { if (event.currentTarget.value !== to) onTo(event.currentTarget.value); }} />
    </label>
  </div>;
}

function TaskStatus({ conversation, from, to, connected, stage, result, error }: {
  conversation: Conversation; from: string; to: string; connected: boolean;
  stage: ExportStage; result: ExportResult | null; error: string;
}) {
  const completed = stage === 'done';
  return <aside className="task-area" aria-label="当前任务">
    <h3><ListBullets size={21} aria-hidden="true" />当前任务</h3>
    <ol className="task-list">
      <li className={connected ? 'task complete' : 'task'}>
        <span className="task-icon">{connected ? <Check size={17} weight="bold" /> : <Circle size={21} />}</span>
        <div><strong>{connected ? '已连接 weflow serve' : '未连接 weflow serve'}</strong></div><time>14:10</time>
      </li>
      <li className="task complete">
        <span className="task-icon"><Check size={17} weight="bold" /></span>
        <div><strong>已载入{conversation.name}的{conversation.category === 'chat' ? '聊天记录' : '记录'}</strong><small>({from || '不限'} ~ {to || '不限'})</small></div><time>14:11</time>
      </li>
      <li className={'task last ' + (stage === 'error' ? 'error' : completed ? 'complete' : '')}>
        <span className="task-icon">{stage === 'exporting' ? <CircleNotch size={25} className="spin" /> : completed ? <Check size={17} weight="bold" /> : <Circle size={25} />}</span>
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
  const exportGeneration = useRef(0);

  const list = conversations.filter(item => item.category === activeCategory);
  const selected = list.find(item => item.id === selectedIds[activeCategory]) || list[0];
  const media = selected.messages.filter(message => message.kind !== 'text');
  const results = useMemo(() => submittedQuery === null ? [] : selected.messages.filter(message => {
    const matchesDate = !searchDatesOpen || ((!from || message.date >= from) && (!to || message.date <= to));
    return matchesDate && (message.text + ' ' + message.sender).toLocaleLowerCase().includes(submittedQuery.toLocaleLowerCase());
  }), [selected, submittedQuery, searchDatesOpen, from, to]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(''), 2800);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => () => {
    if (exportResult) URL.revokeObjectURL(exportResult.downloadUrl);
  }, [exportResult]);

  useEffect(() => {
    if (!highlighted) return;
    document.getElementById('message-' + highlighted)?.scrollIntoView({ block: 'center', behavior: reducedMotion ? 'auto' : 'smooth' });
    const timer = window.setTimeout(() => setHighlighted(null), 2800);
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

  useEffect(() => {
    const dialog = dialogRef.current;
    if (preview && dialog && !dialog.open) dialog.showModal();
    else if (!preview && dialog?.open) dialog.close();
  }, [preview]);

  const resetExport = () => {
    exportGeneration.current += 1;
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
    const generation = ++exportGeneration.current;
    setStage('exporting'); setError(''); setExportResult(null);
    try {
      const result = await exportConversation(selected, { format, from, to, includeMedia });
      if (generation === exportGeneration.current) { setExportResult(result); setStage('done'); }
      else URL.revokeObjectURL(result.downloadUrl);
    } catch (problem) {
      if (generation === exportGeneration.current) { setError(problem instanceof Error ? problem.message : '导出失败，请重试。'); setStage('error'); }
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

  return <main className={'workspace' + (reducedMotion ? ' reduced-motion' : '')}>
    <aside className="sidebar">
      <nav aria-label="主导航">
        {navigation.map(item => {
          const Icon = item.icon;
          return <button key={item.id} type="button" className={'nav-item' + (section === item.id ? ' selected' : '')} aria-current={section === item.id ? 'page' : undefined} aria-label={item.label} onClick={() => selectSection(item.id)}>
            <span className={'nav-icon' + (item.id === 'chat' && section === 'chat' ? ' chat-active' : '')}><Icon size={33} weight="regular" aria-hidden="true" /></span><span className="nav-label">{item.label}</span>
          </button>;
        })}
      </nav>
      <div className={'connection ' + backend.state} role="status" title={backendDetail(backend)}><span className="connection-dot" />{backendLabels[backend.state]}</div>
    </aside>

    <div className="main-area">
      {section === 'settings' ? <section className="browser-panel settings-panel" aria-label="设置">
        <div className="settings-heading"><Gear size={30} /><h1>设置</h1></div>
        <p className="muted">管理连接和浏览偏好。</p>
        <div className="setting-row"><div><h2>weflow serve</h2><p>{backendDetail(backend)}</p></div><button className="small-button" type="button" onClick={recheckBackend}>重新检测</button></div>
        <div className="setting-row"><div><h2>网页版本</h2><p>需要 API {WEB.apiVersion}，weflow {WEB.minBackendVersion} 或更新的版本。</p></div><span className="source-name"><Plugs size={21} />{WEB.version}</span></div>
        <div className="setting-row"><div><h2>默认附带媒体文件</h2><p>导出时同时保存照片等媒体。</p></div><label className="toggle"><input type="checkbox" aria-label="默认附带媒体文件" checked={includeMedia} onChange={event => setIncludeMedia(event.target.checked)} /><span /></label></div>
        <div className="setting-row"><div><h2>减少动画</h2><p>使用更直接的界面切换。</p></div><label className="toggle"><input type="checkbox" aria-label="减少动画" checked={reducedMotion} onChange={event => setReducedMotion(event.target.checked)} /><span /></label></div>
        <small className="settings-footnote">聊天记录目前是示例数据，页面接入 API 后替换。</small>
      </section> : <section className={'browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label={categoryLabels[activeCategory] + '记录浏览'}>
        <div className="conversation-list" aria-label={categoryLabels[activeCategory] + '列表'}>
          {list.map(conversation => <button type="button" key={conversation.id} className={'conversation-row' + (conversation.id === selected.id ? ' active' : '')} aria-label={'选择' + conversation.name} aria-pressed={conversation.id === selected.id} onClick={() => selectConversation(conversation)}>
            <Avatar conversation={conversation} />
            <span className="conversation-copy"><span className="conversation-name">{conversation.name}</span><span className="conversation-preview">{conversation.preview}</span></span>
            <time className="conversation-time">{conversation.stamp}</time>
          </button>)}
        </div>
        <div className="message-pane" aria-label={selected.name + '的记录'}>
          <button type="button" className="mobile-back icon-button" aria-label="返回对话列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={22} /></button>
          <div className="messages">
            {selected.messages.map((message, index) => <div key={message.id}>
              {(index === 0 || selected.messages[index - 1].date !== message.date) && <div className="date-separator">{formatDay(message.date)}</div>}
              <div id={'message-' + message.id} className={'message-row ' + message.direction + (highlighted === message.id ? ' highlighted' : '')}>
                {message.kind === 'image' && message.asset
                  ? <button type="button" className="photo-button" aria-label={'预览' + message.text} onClick={() => setPreview({ type: 'image', src: message.asset!, title: message.text })}><img className="message-photo" src={message.asset} alt={message.text} /></button>
                  : message.kind === 'file'
                    ? <button className="message-bubble file-bubble" type="button" onClick={() => setPreview({ type: 'file', title: message.fileName || message.text })}><FilePdf size={32} /><span>{message.fileName}<small>示例文件</small></span></button>
                    : <div className="message-bubble">{message.text}</div>}
                <time className="message-time">{message.time}</time>
              </div>
            </div>)}
          </div>
        </div>
      </section>}

      <section className="console-panel" aria-label="操作控制台">
        <header className="console-header"><h2><SlidersHorizontal size={27} aria-hidden="true" />操作控制台</h2>
          <div className="function-group" role="tablist" aria-label="控制台功能" onKeyDown={moveConsoleFocus}>
            <span className={'function-indicator ' + (mode === 'export' ? 'at-export' : mode === 'search' ? 'at-search' : 'at-more')} aria-hidden="true" />
            <button type="button" role="tab" id="export-tab" aria-selected={mode === 'export'} aria-controls="console-content" onClick={() => { setMode('export'); setMoreOpen(false); }}>导出</button>
            <button type="button" role="tab" id="search-tab" aria-selected={mode === 'search'} aria-controls="console-content" onClick={() => { setMode('search'); setMoreOpen(false); }}>搜索</button>
            <div className="more-wrapper" ref={moreRef}><button type="button" role="tab" id="more-tab" aria-selected={mode === 'info' || mode === 'media'} aria-haspopup="menu" aria-expanded={moreOpen} aria-controls="console-content" onClick={() => setMoreOpen(value => !value)}>更多<CaretDown size={18} aria-hidden="true" /></button>
              {moreOpen && <div className="more-menu" role="menu" aria-label="更多功能"><button type="button" role="menuitem" onClick={() => { setMode('info'); setMoreOpen(false); }}><Article size={19} />会话资料</button><button type="button" role="menuitem" onClick={() => { setMode('media'); setMoreOpen(false); }}><ImageSquare size={19} />媒体文件</button></div>}
            </div>
          </div>
        </header>

        <div id="console-content" role="tabpanel" aria-labelledby={mode === 'export' ? 'export-tab' : mode === 'search' ? 'search-tab' : 'more-tab'} className={'console-body ' + mode} key={mode}>
          {mode === 'export' ? <>
            <div className="export-area"><div className="function-title"><h3>导出{selected.name}的{selected.category === 'chat' ? '聊天记录' : '记录'}</h3><span className="sample-badge">示例数据</span></div>
              <div className="form-row"><span className="field-label">时间范围</span><DateRange from={from} to={to} onFrom={value => { setFrom(value); resetExport(); }} onTo={value => { setTo(value); resetExport(); }} /></div>
              <div className="form-row format-row"><span className="field-label">导出格式</span><div className="format-group" role="group" aria-label="导出格式"><span className={'format-indicator at-' + format.toLocaleLowerCase()} aria-hidden="true" />{(['HTML', 'JSON', 'Excel'] as const).map(value => <button type="button" key={value} aria-pressed={format === value} onClick={() => { setFormat(value); resetExport(); }}>{value}</button>)}</div>
                <label className="media-checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={includeMedia} onChange={event => { setIncludeMedia(event.target.checked); resetExport(); }} /><span><strong>附带媒体文件</strong><small>图片、语音、视频与表情</small></span></label>
              </div>
              <button type="button" className="primary-button export-button" disabled={stage === 'exporting'} onClick={exportRecords}>{stage === 'exporting' ? <CircleNotch size={25} className="spin" /> : <DownloadSimple size={25} />}<span>{stage === 'exporting' ? '正在导出' : stage === 'done' ? '再次导出' : '开始导出'}</span></button>
            </div>
            <TaskStatus conversation={selected} from={from} to={to} connected={connected} stage={stage} result={exportResult} error={error} />
          </> : mode === 'search' ? <div className="search-area">
            <div className="function-title"><h3>搜索{selected.name}的消息</h3><span className="sample-badge">示例数据</span></div>
            <form className="search-form" onSubmit={search}><div className="search-input"><MagnifyingGlass size={22} aria-hidden="true" /><input type="search" aria-label="搜索此会话的消息" placeholder="搜索此会话的消息" value={query} onChange={event => setQuery(event.target.value)} /><button className={'icon-button' + (searchDatesOpen ? ' on' : '')} type="button" aria-label="筛选搜索日期" aria-expanded={searchDatesOpen} onClick={() => setSearchDatesOpen(value => !value)}><CalendarBlank size={22} /></button></div><button className="primary-button search-button" type="submit"><MagnifyingGlass size={20} />搜索</button></form>
            {searchDatesOpen && <div className="search-date-row"><span className="field-label">时间范围</span><DateRange from={from} to={to} onFrom={setFrom} onTo={setTo} /></div>}
            <div className="search-results" aria-live="polite">{submittedQuery === null
              ? <p className="empty-text">输入关键词，搜索当前会话的记录。</p>
              : results.length
                ? <><p className="results-count">找到 {results.length} 条记录<span>点击可定位到消息</span></p>{results.map(message => <button className="search-result" type="button" key={message.id} onClick={() => setHighlighted(message.id)}><span className="search-result-copy"><strong>{message.sender}</strong><span>{message.text}</span></span><time>{message.date} {message.time}</time><ArrowSquareOut size={17} /></button>)}</>
                : <p className="empty-text">没有找到匹配的消息，试试其他关键词或时间范围。</p>}</div>
          </div> : mode === 'info' ? <div className="info-area">
            <div className="function-title"><h3>会话资料</h3><span className="sample-badge">示例数据</span></div>
            <div className="info-content"><Avatar conversation={selected} /><div><h4>{selected.name}</h4><p>{selected.description}</p></div></div>
            <dl className="info-grid"><div><dt>类型</dt><dd>{categoryLabels[selected.category]}</dd></div><div><dt>记录条数</dt><dd>{selected.messages.length} 条</dd></div><div><dt>媒体文件</dt><dd>{media.length} 个</dd></div><div><dt>时间范围</dt><dd>{selected.messages[0]?.date} ↔ {selected.messages.at(-1)?.date}</dd></div></dl>
          </div> : <div className="media-area">
            <div className="function-title"><h3>{selected.name}的媒体文件</h3><span className="sample-badge">示例数据</span></div>
            <div className="media-list">{media.length ? media.map(message => <button className="media-item" type="button" key={message.id} onClick={() => message.asset ? setPreview({ type: 'image', src: message.asset, title: message.text }) : setPreview({ type: 'file', title: message.fileName || message.text })}>{message.asset ? <img src={message.asset} alt="" /> : <FilePdf size={40} />}<span>{message.fileName || message.text}<small>{message.date} {message.time}</small></span><ArrowSquareOut size={20} /></button>) : <p className="empty-text">这段记录中没有媒体文件。</p>}</div>
          </div>}
        </div>
      </section>
    </div>

    <dialog className="preview-dialog" ref={dialogRef} aria-label={preview?.title || '媒体预览'} onCancel={() => setPreview(null)} onClick={event => { if (event.target === event.currentTarget) setPreview(null); }}>
      {preview && <div className="preview-content"><header><strong>{preview.title}</strong><button type="button" className="icon-button" aria-label="关闭预览" onClick={() => setPreview(null)}><X size={25} /></button></header>{preview.type === 'image' ? <img src={preview.src} alt={preview.title} /> : <div className="file-preview"><FilePdf size={70} /><h3>{preview.title}</h3><p>示例文件记录，可查看名称与记录时间。</p></div>}</div>}
    </dialog>
    {toast && <div className="toast" role="status"><CheckCircle size={19} />{toast}</div>}
  </main>;
}
