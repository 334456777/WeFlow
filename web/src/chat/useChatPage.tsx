// The chat page (#196): the conversation list, the messages of the one open, and its cards in the console: export
// (#211), search (#199), details and media (#200). Until a server is connected it shows the sample records instead,
// for the Moments and WeChat Pay pages as well, so the page can be seen without one.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { ArrowSquareOut, Check, DownloadSimple, FilePdf, MagnifyingGlass } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { lookupMessage, type Resource } from '../api/details';
import { jobFileUrl } from '../api/jobs';
import type { SearchHit } from '../api/messages';
import { Avatar } from '../Avatar';
import { DateRange } from '../calendar/DateRange';
import { parseDay } from '../calendar/dates';
import type { Card } from '../CardStack';
import { FormatChips, formatsLabel, MEDIA_NOTE, packed, type MessageFormat } from '../exports/ExportCards';
import { HoverTrack } from '../HoverTrack';
import { jobDetail } from '../jobs/TaskCenter';
import { findJob, removeJob, runJob } from '../jobs/useJobs';
import { categoryLabels, conversations, CURRENT_DATE, DEFAULT_FROM, formatDay, sampleSessions } from '../mock/data';
import { FEEDBACK_MS, prefersReducedMotion } from '../motion';
import { Scrollbar } from '../Scrollbar';
import type { OpenAt, Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import type { Preferences } from '../shell/preferences';
import { TokenPrompt } from '../TokenForm';
import type { Category, Conversation, ExportResult } from '../types';
import { BizPushes } from './BizPushes';
import { ExportControl, stageOf } from './ExportControl';
import { LiveMessages } from './LiveMessages';
import { MediaPanel } from './MediaPanel';
import { SearchPanel, searchSummary } from './SearchPanel';
import { SessionInfo } from './SessionInfo';
import { SessionList } from './SessionList';
import { formatSessionTime, kindLabels, kindOf } from './sessions';
import { useBizAccounts } from './useBizAccounts';
import type { Jump } from './useMessages';
import { useSearch } from './useSearch';
import type { SessionsState } from './useSessions';

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

export interface ChatPage extends Page {
  view: ReactNode;
  /** Its cards, the export first; the console puts the task centre after it. */
  cards: Card[];
  shown: string[];
  /** Opens a conversation, from another page. */
  open(username: string): void;
  /** Opens a conversation at a message, from another page. */
  openAt: OpenAt;
  /**
   * Another page is opened: the message a jump looks up is let go of (#415); `reset`, unless only the settings are
   * opened, also lets go of the export's job (it goes on in the task centre) and ends the sample's search.
   */
  leave(reset: boolean): void;
}

/** `category`: the records browsed, the chat's or the sample's of the Moments and WeChat Pay pages. */
export function useChatPage(shell: Shell, prefs: Preferences, category: Category): ChatPage {
  const { backend, connected, scope, sessionList, setMobileDetail, openPreview, toast } = shell;
  const { includeMedia, setIncludeMedia, reducedMotion } = prefs;
  // the chat page reads `weflow serve` once it is connected; until then, and for the other pages, sample records
  const live = category === 'chat' && (connected || backend.state === 'checking');
  const [selectedIds, setSelectedIds] = useState<Record<Category, string>>({ chat: 'lin', moments: 'moment-lin', payments: 'payment-coffee' });
  const [from, setFrom] = useState(DEFAULT_FROM);
  const [to, setTo] = useState(CURRENT_DATE);
  // the days of a real export: all of them unless chosen (the sample's days are the sample's)
  const [liveRange, setLiveRange] = useState({ from: '', to: '' });
  const [formats, setFormats] = useState<MessageFormat[]>(['html']);
  // the job of the export card (#211): it runs on the server, in the task centre
  const [exportJobId, setExportJobId] = useState<string | null>(null);
  const [exportError, setExportError] = useState('');
  const [query, setQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState<string | null>(null);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  const exportJob = findJob(shell.jobs, exportJobId);
  const stage = stageOf(exportJob, exportError);
  const exportResult: ExportResult | null = exportJob?.status === 'succeeded' && exportJob.file
    ? { fileName: exportJob.file.name, count: 0, downloadUrl: jobFileUrl(exportJob.id) } : null;
  const [liveId, setLiveId] = useState<string | null>(null);
  // the message a jump looks up (#200, #209): let go of when another page, conversation or account is opened (#415)
  const mediaLookup = useRef<AbortController | null>(null);
  useEffect(() => () => { mediaLookup.current?.abort(); }, [scope]);
  // the official accounts followed without a conversation, once the 「公众号」 filter is first chosen (#203)
  const [officialsWanted, setOfficialsWanted] = useState(false);
  const officials = useBizAccounts(connected && officialsWanted, scope);
  const liveSession = sessionList.find(session => session.username === liveId) ?? officials.find(session => session.username === liveId);
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
  const list = conversations.filter(item => item.category === category);
  const selected = live ? liveConversation(liveSession) : list.find(item => item.id === selectedIds[category]) || list[0];
  const media = selected.messages.filter(message => message.kind !== 'text');
  const results = useMemo(() => submittedQuery === null ? [] : selected.messages.filter(message => {
    const matchesDate = (!from || message.date >= from) && (!to || message.date <= to);
    return matchesDate && (message.text + ' ' + message.sender).toLocaleLowerCase().includes(submittedQuery.toLocaleLowerCase());
  }), [selected, submittedQuery, from, to]);

  useEffect(() => {
    if (!highlighted) return;
    document.getElementById('message-' + highlighted)?.scrollIntoView({ block: 'center', behavior: prefersReducedMotion(reducedMotion) ? 'auto' : 'smooth' });
    const timer = window.setTimeout(() => setHighlighted(null), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [highlighted, reducedMotion]);

  // another conversation or other options: the card lets go of its job, which goes on in the task centre
  const resetExport = () => { setExportJobId(null); setExportError(''); };

  const selectConversation = (id: string) => {
    mediaLookup.current?.abort();
    if (live) {
      setLiveId(id);
      // a search of the conversation left behind is over; one of all conversations still holds
      if (liveSearch.state.query?.sessionId) liveSearch.clear();
    }
    else setSelectedIds(current => ({ ...current, [category]: id }));
    setMobileDetail(true); setSubmittedQuery(null); setQuery(''); resetExport();
  };

  // On a phone the console is below the messages: after a jump, the page goes up to them.
  const showMessages = () => {
    setMobileDetail(true);
    if (!window.matchMedia?.('(max-width: 640px)').matches) return;
    requestAnimationFrame(() => paneRef.current?.closest('.browser-panel')?.scrollIntoView({ block: 'start', behavior: prefersReducedMotion(reducedMotion) ? 'auto' : 'smooth' }));
  };

  const openHit = (hit: SearchHit) => {
    if (!sessionList.some(session => session.username === hit._session_id)) { toast('这条消息所在的会话不在会话列表里。'); return; }
    setLiveId(hit._session_id);
    setJumpTo({ sessionId: hit._session_id, target: { at: 'message', message: hit }, seq: ++jumps.current });
    showMessages();
  };

  // a picture, video or file of the media card (#200), or a chat or mention of another page: its conversation opens
  // at the message
  const openMedia = (sessionId: string, item: Resource) => openMessage(sessionId, item.localId, item.createTime);
  const openMessage = async (sessionId: string, localId: number, createTime: number) => {
    mediaLookup.current?.abort();
    const controller = new AbortController();
    mediaLookup.current = controller;
    try {
      const message = await lookupMessage(sessionId, localId, createTime, controller.signal);
      if (controller.signal.aborted) return;
      if (!message?._cursor) { toast('没有找到这条消息。'); return; }
      setJumpTo({ sessionId, target: { at: 'message', message }, seq: ++jumps.current });
      showMessages();
    } catch {
      if (!controller.signal.aborted) toast('没有找到这条消息。');
    }
  };

  const jumpToDay = (sessionId: string, day: string) => {
    setJumpTo({ sessionId, target: { at: 'time', seconds: parseDay(day).getTime() / 1000 }, seq: ++jumps.current });
    showMessages();
  };

  const exportRecords = async () => {
    if (!liveSession || !formats.length) return;
    setExportError(''); setExportJobId(null);
    try {
      const job = await runJob('导出 ' + selected.name + ' · ' + formatsLabel(formats), 'POST', 'export/messages', {
        'session-id': liveSession.username, format: formats, start: liveRange.from || undefined, end: liveRange.to || undefined,
        media: includeMedia ? 'all' : undefined, zip: packed(formats, includeMedia) ? true : undefined,
      });
      setExportJobId(job.id);
    } catch (problem) {
      setExportError(problem instanceof Error && problem.message ? problem.message : '导出没有开始，请重试。');
    }
  };
  const cancelExport = () => {
    if (exportJobId) removeJob(exportJobId).catch(() => setExportError('取消失败，请重试。'));
  };

  // an empty search clears the results rather than matching every message (#199), as the live search does
  const search = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSubmittedQuery(query.trim() || null);
  };

  const cards: Card[] = [
    {
      id: 'export', title: '导出', summary: (formats.length ? formatsLabel(formats) : '未选择格式') + (includeMedia ? ' · 含媒体' : ''),
      content: <>
        <p className="card-lead"><span className="clamp-1">{selected.name}的{selected.category === 'chat' ? '聊天记录' : '记录'}</span>{!selected.live && <span className="badge">示例数据</span>}</p>
        <div className="form-row"><span className="field-label">时间范围</span>{selected.live
          ? <DateRange label="导出的时间范围" from={liveRange.from} to={liveRange.to} onChange={range => { setLiveRange(range); resetExport(); }} />
          : <DateRange label="导出的时间范围" from={from} to={to} onChange={range => { setFrom(range.from); setTo(range.to); resetExport(); }} />}</div>
        <div className="form-row format-row"><span className="field-label">导出格式</span>
          <FormatChips formats={formats} onChange={next => { setFormats(next); resetExport(); }} />
          <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={includeMedia} onChange={event => { setIncludeMedia(event.target.checked); resetExport(); }} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>附带媒体文件 <small>{MEDIA_NOTE}</small></span></label>
        </div>
        {selected.live && liveSession
          ? <><ExportControl stage={stage} result={exportResult} ready={formats.length > 0} onStart={exportRecords} onCancel={cancelExport} />
            {stage === 'error' && <p className="card-note" role="alert">{exportJob ? jobDetail(exportJob) : exportError}</p>}
            {stage === 'exporting' && exportJob && <p className="card-note" aria-live="polite">{exportJob.status === 'queued' ? '排队中：前面的任务完成后开始。' : jobDetail(exportJob)}</p>}
            {stage === 'done' && packed(formats, includeMedia) && <p className="card-note">{formats.length > 1 ? '各个格式的文件' : '导出文件'}{includeMedia ? '和 media 文件夹' : ''}打包在这个 zip 里。</p>}</>
          : <><button type="button" className="primary-button large export-pending" disabled><DownloadSimple size={18} />开始导出</button><p className="card-note">{selected.live ? '选择一个会话后导出。' : '连接 weflow serve 后，可以导出本机的真实记录。'}</p></>}
      </>,
    },
    {
      id: 'search', title: '搜索', summary: selected.live ? searchSummary(liveSearch.state) : submittedQuery === null ? '当前会话' : results.length + ' 条结果',
      content: selected.live
        ? <SearchPanel key={scope} session={liveSession} sessions={sessionList} search={liveSearch} onOpenHit={openHit} onJumpDay={jumpToDay} />
        : <>
        <form className="search-form" onSubmit={search}>
          <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" /><input type="search" aria-label="搜索此会话的消息" placeholder={'搜索' + selected.name + '的消息'} value={query} onChange={event => setQuery(event.target.value)} /></div>
          <button className="primary-button search-button" type="submit" aria-label="搜索"><MagnifyingGlass size={16} /><span className="search-button-label">搜索</span></button>
        </form>
        <div className="search-date-row"><span className="field-label">时间范围</span><DateRange label="搜索的时间范围" from={from} to={to} onChange={range => { setFrom(range.from); setTo(range.to); }} /></div>
        <div className="search-results" aria-live="polite" data-roving="vertical" data-roving-items=".search-result"><HoverTrack items=".search-result" />{submittedQuery === null
          ? <p className="empty-text">输入关键词，搜索当前会话的记录。</p>
          : results.length
            ? <><p className="results-count">找到 {results.length} 条记录<span>点击可定位到消息</span></p>{results.map(message => <button className="search-result" type="button" key={message.id} onClick={() => setHighlighted(message.id)}><span className="search-result-copy"><strong>{message.sender}</strong><span className="clamp-1">{message.text}</span></span><time>{message.date} {message.time}</time><ArrowSquareOut size={14} /></button>)}</>
            : <p className="empty-text">没有找到匹配的消息，试试其他关键词或时间范围。</p>}</div>
      </>,
    },
    {
      id: 'info', title: '会话资料', summary: selected.live ? (selected.messageCount ? '约 ' + selected.messageCount + ' 条' : '') : selected.messages.length + ' 条记录',
      content: selected.live
        ? (open: boolean) => liveSession ? <SessionInfo key={scope + ':' + liveSession.username} session={liveSession} active={open} /> : null
        : <>
          <div className="info-content"><Avatar name={selected.name} url={selected.avatar} /><div><h4>{selected.name}</h4><p className="clamp-2">{selected.description}</p></div></div>
          <dl className="info-grid"><div><dt>类型</dt><dd>{categoryLabels[selected.category]}</dd></div><div><dt>记录条数</dt><dd className="num">{selected.messages.length} 条</dd></div><div><dt>媒体文件</dt><dd className="num">{media.length} 个</dd></div><div className="info-wide"><dt>时间范围</dt><dd className="num">{selected.messages[0]?.date} ↔ {selected.messages.at(-1)?.date}</dd></div></dl>
        </>,
    },
    {
      id: 'media', title: '媒体文件', summary: selected.live ? '图片、视频与文件' : media.length + ' 个',
      content: selected.live
        ? (open: boolean) => liveSession ? <MediaPanel key={scope + ':' + liveSession.username} sessionId={liveSession.username} active={open} onOpen={item => openMedia(liveSession.username, item)} /> : null
        : <div className="media-list" data-roving="grid" data-roving-items=".media-item"><HoverTrack items=".media-item" />{media.length ? media.map(message => <button className="media-item" type="button" key={message.id} onClick={() => message.asset ? openPreview({ type: 'image', src: message.asset, title: message.text }) : openPreview({ type: 'file', title: message.fileName || message.text })}>{message.asset ? <img src={message.asset} alt="" /> : <FilePdf size={32} />}<span><span className="clamp-1">{message.fileName || message.text}</span><small>{message.date} {message.time}</small></span><ArrowSquareOut size={14} /></button>) : <p className="empty-text">这段记录中没有媒体文件。</p>}</div>,
    },
  ];
  // nothing to show of a real conversation until one is open
  const shown = ['export', 'tasks', 'search', 'info', 'media'].filter(id => !(selected.live && !liveSession && (id === 'info' || id === 'media')));

  const view = <BrowserPanel label={categoryLabels[category] + '记录浏览'} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)}
    back="返回对话列表" title={selected.live ? liveSession?.displayName ?? '' : selected.name} paneLabel={selected.name + '的记录'} pane={paneRef}
    list={category === 'chat'
      ? backend.state === 'unauthorized'
        ? <TokenPrompt />
        : <SessionList state={connected ? shell.sessions : backend.state === 'checking' ? CHECKING : SAMPLE_SESSIONS} sample={!connected && backend.state !== 'checking'} selectedId={selected.id || null} onSelect={selectConversation} onRetry={shell.reloadSessions}
          officials={officials} onFilter={filter => { if (filter === 'official') setOfficialsWanted(true); }} />
      : <><div ref={listRef} className="conversation-list scroll-area" aria-label={categoryLabels[category] + '列表'} data-roving="vertical" data-roving-items=".conversation-row"><HoverTrack items=".conversation-row" />
        {list.map(conversation => <button type="button" key={conversation.id} className={'conversation-row' + (conversation.id === selected.id ? ' active' : '')} aria-label={'选择' + conversation.name} aria-pressed={conversation.id === selected.id} onClick={() => selectConversation(conversation.id)}>
          <Avatar name={conversation.name} url={conversation.avatar} />
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{conversation.name}</span><time className="conversation-time">{conversation.stamp}</time></span>
            <span className="conversation-preview clamp-1">{conversation.preview}</span>
          </span>
        </button>)}
      </div><Scrollbar target={listRef} /></>}>
    {selected.live && (liveSession
      ? kindOf(liveSession.username) === 'official'
        ? <BizPushes key={scope + ':' + liveSession.username} session={liveSession} />
        : <LiveMessages key={scope} session={liveSession} scroller={paneRef} jump={jumpTo} onPreview={openPreview} />
      : <div className="pane-message">{shell.sessions.status === 'ready' && <p>选择左侧的一个会话。</p>}</div>)}
    {!selected.live && <div className="messages">
      {selected.messages.map((message, index) => <div key={message.id}>
        {(index === 0 || selected.messages[index - 1].date !== message.date) && <div className="date-separator">{formatDay(message.date)}</div>}
        <div id={'message-' + message.id} className={'message-row ' + message.direction + (highlighted === message.id ? ' highlighted' : '')}>
          {message.kind === 'image' && message.asset
            ? <button type="button" className="photo-button" aria-label={'预览' + message.text} onClick={() => openPreview({ type: 'image', src: message.asset!, title: message.text })}><img className="message-photo" src={message.asset} alt={message.text} /></button>
            : message.kind === 'file'
              ? <button className="message-bubble file-bubble" type="button" onClick={() => openPreview({ type: 'file', title: message.fileName || message.text })}><FilePdf size={28} /><span>{message.fileName}<small>示例文件</small></span></button>
              : <div className="message-bubble">{message.text}</div>}
          <time className="message-time">{message.time}</time>
        </div>
      </div>)}
    </div>}
  </BrowserPanel>;

  return {
    view, cards, shown,
    open: setLiveId,
    openAt: (sessionId, localId, createTime) => { setLiveId(sessionId); void openMessage(sessionId, localId, createTime); },
    leave: reset => {
      mediaLookup.current?.abort();
      if (reset) { resetExport(); setSubmittedQuery(null); setQuery(''); }
    },
  };
}
