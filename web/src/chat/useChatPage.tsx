// The chat page (#196): the conversation list, the messages of the one open, and its cards in the console: export
// (#211), search (#199), details and media (#200). The records are the server's, or the sample's while no server has
// answered (#879), read and shown the same way; once a server was connected, what was read from it stays while it is
// gone.
import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { DownloadSimple } from '@phosphor-icons/react';
import { lookupMessage, type Resource } from '../api/details';
import { jobFileUrl } from '../api/jobs';
import type { SearchHit } from '../api/messages';
import { DateRange } from '../calendar/DateRange';
import { parseDay } from '../calendar/dates';
import type { Card } from '../CardStack';
import { FormatChips, formatsLabel, MediaChips, mediaLabel, packed, type MessageFormat } from '../exports/ExportCards';
import { FormatLanes } from '../exports/FormatLanes';
import { jobDetail } from '../jobs/TaskCenter';
import { findJob, removeJob, runJob } from '../jobs/useJobs';
import { prefersReducedMotion } from '../motion';
import type { OpenAt, Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import type { Preferences } from '../shell/preferences';
import { TokenPrompt } from '../TokenForm';
import type { ExportResult } from '../types';
import { BizPushes } from './BizPushes';
import { ExportControl, stageOf } from './ExportControl';
import { LiveMessages } from './LiveMessages';
import { MediaPanel } from './MediaPanel';
import { SearchPanel, searchSummary } from './SearchPanel';
import { SessionInfo } from './SessionInfo';
import { SessionList } from './SessionList';
import { kindOf } from './sessions';
import { useBizAccounts } from './useBizAccounts';
import type { Jump } from './useMessages';
import { useSearch } from './useSearch';
import type { SessionsState } from './useSessions';
import { t } from '../i18n';
import './chat.css';

// Until /health has answered, no records: neither the sample nor a server's flashes up before the other.
const CHECKING: SessionsState = { status: 'loading' };

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
   * opened, also lets go of the export's job (it goes on in the task centre).
   */
  leave(reset: boolean): void;
}

export function useChatPage(shell: Shell, prefs: Preferences): ChatPage {
  const { backend, records, sample, scope, sessionList, setMobileDetail, openPreview, toast } = shell;
  const { media: mediaKinds, setMedia: setMediaKinds, reducedMotion } = prefs;
  const includeMedia = mediaKinds.length > 0;
  // the days of an export: all of them unless chosen
  const [range, setRange] = useState({ from: '', to: '' });
  const [formats, setFormats] = useState<MessageFormat[]>(['html']);
  // the job of the export card (#211): it runs on the server, in the task centre
  const [exportJobId, setExportJobId] = useState<string | null>(null);
  const [exportError, setExportError] = useState('');
  const paneRef = useRef<HTMLDivElement>(null);
  const exportJob = findJob(shell.jobs, exportJobId);
  const stage = stageOf(exportJob, exportError);
  const exportResult: ExportResult | null = exportJob?.status === 'succeeded' && exportJob.file
    ? { fileName: exportJob.file.name ?? '', count: 0, downloadUrl: jobFileUrl(exportJob.id) } : null;
  const [openId, setOpenId] = useState<string | null>(null);
  // the message a jump looks up (#200, #209): let go of when another page, conversation or account is opened (#415)
  const mediaLookup = useRef<AbortController | null>(null);
  useEffect(() => () => { mediaLookup.current?.abort(); }, [scope]);
  // the official accounts followed without a conversation, once the 「公众号」 filter is first chosen (#203)
  const [officialsWanted, setOfficialsWanted] = useState(false);
  const officials = useBizAccounts(records && officialsWanted, scope);
  const session = sessionList.find(s => s.username === openId) ?? officials.find(s => s.username === openId);
  const name = session ? session.displayName || session.username : '';
  const search = useSearch();
  // A day picked or a search hit opened: its conversation shows the messages there (#199).
  const [jumpTo, setJumpTo] = useState<Jump | null>(null);
  const jumps = useRef(0);
  const { clear: clearSearch } = search;
  useEffect(() => {
    if (!scope) return;
    setJumpTo(null);
    clearSearch();
  }, [scope, clearSearch]);
  useEffect(() => {
    if (!session && sessionList.length) setOpenId(sessionList[0].username);
  }, [session, sessionList]);

  // another conversation or other options: the card lets go of its job, which goes on in the task centre
  const resetExport = () => { setExportJobId(null); setExportError(''); };

  const selectConversation = (id: string) => {
    mediaLookup.current?.abort();
    setOpenId(id);
    // a search of the conversation left behind is over; one of all conversations still holds
    if (search.state.query?.sessionId) search.clear();
    setMobileDetail(true); resetExport();
  };

  // On a phone the console is below the messages: after a jump, the page goes up to them.
  const showMessages = () => {
    setMobileDetail(true);
    if (!window.matchMedia?.('(max-width: 640px)').matches) return;
    requestAnimationFrame(() => paneRef.current?.closest('.browser-panel')?.scrollIntoView({ block: 'start', behavior: prefersReducedMotion(reducedMotion) ? 'auto' : 'smooth' }));
  };

  const openHit = (hit: SearchHit) => {
    if (!sessionList.some(s => s.username === hit._session_id)) { toast(t('这条消息所在的会话不在会话列表里。')); return; }
    setOpenId(hit._session_id);
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
      if (!message?._cursor) { toast(t('没有找到这条消息。')); return; }
      setJumpTo({ sessionId, target: { at: 'message', message }, seq: ++jumps.current });
      showMessages();
    } catch {
      if (!controller.signal.aborted) toast(t('没有找到这条消息。'));
    }
  };

  const jumpToDay = (sessionId: string, day: string) => {
    setJumpTo({ sessionId, target: { at: 'time', seconds: parseDay(day).getTime() / 1000 }, seq: ++jumps.current });
    showMessages();
  };

  const exportRecords = async () => {
    if (!session || !formats.length) return;
    setExportError(''); setExportJobId(null);
    try {
      const job = await runJob(t('导出 {name} · {formats}', { name, formats: formatsLabel(formats) }), 'POST', 'export/messages', {
        'session-id': session.username, format: formats, start: range.from || undefined, end: range.to || undefined,
        media: includeMedia ? mediaKinds : undefined, zip: packed(formats, includeMedia) ? true : undefined,
      });
      setExportJobId(job.id);
    } catch (problem) {
      setExportError(problem instanceof Error && problem.message ? problem.message : t('导出没有开始，请重试。'));
    }
  };
  const cancelExport = () => {
    if (exportJobId) removeJob(exportJobId).catch(() => setExportError(t('取消失败，请重试。')));
  };

  const count = session?.messageCountHint;
  const cards: Card[] = [
    {
      id: 'export', title: t('导出'), summary: (formats.length ? formatsLabel(formats) : t('未选择格式')) + (includeMedia ? ' · ' + t('含{kinds}', { kinds: mediaLabel(mediaKinds) }) : ''),
      content: <>
        <p className="card-lead"><span className="clamp-1">{session ? t('{name}的聊天记录', { name }) : t('聊天记录')}</span>{sample && <span className="badge">{t('示例数据')}</span>}</p>
        <div className="form-row"><span className="field-label">{t('时间范围')}</span>
          <DateRange label={t('导出的时间范围')} from={range.from} to={range.to} onChange={next => { setRange(next); resetExport(); }} /></div>
        <div className="form-row format-row chips-below"><span className="field-label">{t('导出格式')}</span>
          <FormatChips formats={formats} onChange={next => { setFormats(next); resetExport(); }} />
        </div>
        <div className="form-row format-row chips-below"><span className="field-label">{t('附带媒体')}</span>
          <MediaChips kinds={mediaKinds} label={t('附带的媒体（可多选）')} onChange={next => { setMediaKinds(next); resetExport(); }} />
        </div>
        {session
          ? <><ExportControl stage={stage} result={exportResult} ready={formats.length > 0} onStart={exportRecords} onCancel={cancelExport} />
            {stage === 'error' && <p className="card-note" role="alert">{exportJob ? jobDetail(exportJob) : exportError}</p>}
            {stage === 'exporting' && exportJob && <p className="card-note" aria-live="polite">{exportJob.status === 'queued' ? t('排队中：前面的任务完成后开始。') : jobDetail(exportJob)}</p>}
            {stage === 'exporting' && <FormatLanes progress={exportJob?.progress} />}
            {stage === 'done' && packed(formats, includeMedia) && <p className="card-note">{formats.length > 1
              ? includeMedia ? t('各个格式的文件和 media 文件夹打包在这个 zip 里。') : t('各个格式的文件打包在这个 zip 里。')
              : includeMedia ? t('导出文件和 media 文件夹打包在这个 zip 里。') : t('导出文件打包在这个 zip 里。')}</p>}</>
          : <><button type="button" className="primary-button large export-pending" disabled><DownloadSimple size={18} />{t('开始导出')}</button><p className="card-note">{t('选择一个会话后导出。')}</p></>}
      </>,
    },
    {
      id: 'search', title: t('搜索'), summary: searchSummary(search.state),
      content: <SearchPanel key={scope} session={session} sessions={sessionList} search={search} onOpenHit={openHit} onJumpDay={jumpToDay} />,
    },
    {
      id: 'info', title: t('会话资料'), summary: count ? t('约 {n} 条', { n: count }) : '',
      content: (open: boolean) => session ? <SessionInfo key={scope + ':' + session.username} session={session} active={open} /> : null,
    },
    {
      id: 'media', title: t('媒体文件'), summary: t('图片、视频与文件'),
      content: (open: boolean) => session ? <MediaPanel key={scope + ':' + session.username} sessionId={session.username} active={open} onOpen={item => openMedia(session.username, item)} /> : null,
    },
  ];
  // nothing to show of a conversation until one is open
  const shown = ['export', 'tasks', 'search', 'info', 'media'].filter(id => session || (id !== 'info' && id !== 'media'));

  const view = <BrowserPanel label={t('{category}记录浏览', { category: t('聊天') })} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)}
    back={t('返回对话列表')} title={name} paneLabel={t('{name}的记录', { name })} pane={paneRef} detailKey={session?.username ?? null}
    list={backend.state === 'unauthorized'
      ? <TokenPrompt />
      : <SessionList state={records ? shell.sessions : CHECKING} sample={sample} selectedId={session?.username ?? null} onSelect={selectConversation} onRetry={shell.reloadSessions}
        officials={officials} onFilter={filter => { if (filter === 'official') setOfficialsWanted(true); }} />}>
    {session
      ? kindOf(session.username) === 'official'
        ? <BizPushes key={scope + ':' + session.username} session={session} />
        : <LiveMessages key={scope} session={session} scroller={paneRef} jump={jumpTo} onPreview={openPreview} />
      : <div className="pane-message">{shell.sessions.status === 'ready' && <p>{t('选择左侧的一个会话。')}</p>}</div>}
  </BrowserPanel>;

  return {
    view, cards, shown,
    open: setOpenId,
    openAt: (sessionId, localId, createTime) => { setOpenId(sessionId); void openMessage(sessionId, localId, createTime); },
    leave: reset => {
      mediaLookup.current?.abort();
      if (reset) resetExport();
    },
  };
}
