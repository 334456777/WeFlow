// The workspace from the #130 prototype: navigation on the left, records in the middle, the operations
// console on the right as in Claude.ai (#130).
// Records are still sample data (src/mock); the connection state is the real `weflow serve`.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import {
  AddressBook, Aperture, ArrowClockwise, ArrowLeft, ArrowSquareOut, CalendarBlank,
  ChatCircleDots, Check, CheckCircle, CircleNotch,
  DownloadSimple, Export, FilePdf, Gear, MagnifyingGlass, Plugs, SidebarSimple, Wallet, X,
} from '@phosphor-icons/react';
import { useBackend, WEB, type Backend, type Health } from './api/backend';
import { onAuth, setToken, token } from './api/client';
import { TokenForm, TokenPrompt } from './TokenForm';
import { useScope } from './api/scope';
import type { ChatSession } from './api/chat';
import { Avatar } from './Avatar';
import { DateRange } from './calendar/DateRange';
import { CardStack, type Card } from './CardStack';
import { SessionList } from './chat/SessionList';
import { formatSessionTime, kindLabels, kindOf } from './chat/sessions';
import { useSessions, type SessionsState } from './chat/useSessions';
import { BizPushes } from './chat/BizPushes';
import { LiveMessages } from './chat/LiveMessages';
import { useBizAccounts } from './chat/useBizAccounts';
import { SearchPanel, searchSummary } from './chat/SearchPanel';
import { SessionInfo } from './chat/SessionInfo';
import { MediaPanel } from './chat/MediaPanel';
import { lookupMessage, type Resource } from './api/details';
import type { Jump } from './chat/useMessages';
import { useSearch } from './chat/useSearch';
import { parseDay } from './calendar/dates';
import type { SearchHit } from './api/messages';
import { PreviewImage, PreviewVideo, type Preview } from './chat/MessageView';
import { categoryLabels, conversations, CURRENT_DATE, DEFAULT_FROM, formatDay, sampleSessions } from './mock/data';
import { FEEDBACK_MS, usePresence } from './motion';
import { HoverTrack } from './HoverTrack';
import { Scrollbar } from './Scrollbar';
import { Segmented } from './Segmented';
import { useTheme, type ThemeChoice } from './theme';
import { NO_FILTER, type SnsFilter } from './api/sns';
import { MomentsFeed } from './moments/MomentsFeed';
import { MomentPeople } from './moments/MomentPeople';
import { filterSummary, MomentsFilter, MomentsStats } from './moments/MomentsCards';
import { useMomentPeople } from './moments/useMomentPeople';
import { PaymentList } from './payments/PaymentList';
import { PaymentsSkeleton } from './Skeleton';
import { PaymentMerchants } from './payments/PaymentMerchants';
import { payFilterSummary, PaymentsExport, PaymentsFilter, PaymentsSummary } from './payments/PaymentCards';
import { ALL_PAYMENTS, merchantsOf, usePayments, withinDays, type PayFilter } from './payments/usePayments';
import type { Category, Conversation, ExportResult, Navigation } from './types';
import { jobFileUrl, type Job } from './api/jobs';
import { jobDetail, jobsSummary, outputOf, TaskCenter } from './jobs/TaskCenter';
import { findJob, removeJob, runJob, useJobs } from './jobs/useJobs';
import { batchSummary, BatchExport, DEFAULT_BATCH, formatLabel, MESSAGE_FORMATS, SessionsExport, type BatchOptions, type MessageFormat } from './exports/ExportCards';
import { ExportPicker, PickedList } from './exports/ExportPicker';
import type { ContactKind } from './api/contacts';
import { ContactList } from './contacts/ContactList';
import { ContactDetail } from './contacts/ContactDetail';
import { ContactsExport, contactsExportSummary, ContactsSummary, DEFAULT_CONTACTS_EXPORT } from './contacts/ContactCards';
import { useContacts } from './contacts/useContacts';
import { currentProfile } from './api/setup';
import { SetupWizard } from './settings/SetupWizard';
import { Accounts, AccountsSkeleton } from './settings/Accounts';
import { AdvancedTools } from './settings/AdvancedTools';
import { CommandConsole } from './settings/CommandConsole';
import { AiSetting, CacheSetting, LanguageSetting } from './settings/MoreSettings';
import { dismissSetup, setupDismissed, useProfiles } from './settings/useProfiles';

type ExportStage = 'idle' | 'exporting' | 'done' | 'error';
// Previews of the sample records and of real messages (images and videos of #198).

const navigation = [
  { id: 'chat', label: '聊天', icon: ChatCircleDots },
  { id: 'moments', label: '朋友圈', icon: Aperture },
  { id: 'payments', label: '微信支付', icon: Wallet },
  { id: 'contacts', label: '通讯录', icon: AddressBook },
  { id: 'export', label: '导出', icon: Export },
  { id: 'settings', label: '设置', icon: Gear },
] as const;


const backendLabels: Record<Backend['state'], string> = {
  checking: '正在连接', ready: '已连接', offline: '未连接', incompatible: '版本不兼容', unauthorized: '需要令牌',
};

function backendDetail(backend: Backend): string {
  switch (backend.state) {
    case 'checking': return '正在检查 weflow serve。';
    case 'offline': return '没有找到 weflow serve，请先运行 weflow serve。';
    case 'incompatible': return backend.reason + '。';
    case 'unauthorized': return 'weflow serve 在运行，但拒绝了这个标签页的令牌。';
    case 'ready': return '已连接 weflow serve。';
  }
}

/** How the server runs (#214): its limits and the insight engine, when it says them. */
function serverDetail(health: Health): string | null {
  const { limits, insight } = health;
  if (!limits && insight === undefined) return null;
  const parts: string[] = [];
  if (limits?.maxTasks !== undefined) parts.push('同时运行任务 ' + limits.maxTasks + ' 个');
  if (limits?.maxReads !== undefined) parts.push('读取 ' + limits.maxReads + ' 个');
  if (limits?.maxMedia !== undefined) parts.push('媒体解码 ' + limits.maxMedia + ' 个');
  if (insight !== undefined) parts.push(insight ? '灵感引擎已开启' : '灵感引擎未开启');
  return parts.join(' · ');
}

/** How long a connection stays 「已连接」 before its notice fades away. */
const STEADY_MS = 3000;
const THEMES: { value: ThemeChoice; label: string }[] = [{ value: 'system', label: '跟随系统' }, { value: 'light', label: '浅色' }, { value: 'dark', label: '深色' }];
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

/** Where the export of the card stands, from its job. */
function stageOf(job: Job | undefined, error: string): ExportStage {
  if (!job) return error ? 'error' : 'idle';
  if (job.status === 'queued' || job.status === 'running') return 'exporting';
  return job.status === 'succeeded' ? 'done' : job.status === 'failed' ? 'error' : 'idle';
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
  // whether this tab holds a token (#214); never its value
  const [tokenSaved, setTokenSaved] = useState(() => Boolean(token()));
  useEffect(() => onAuth(event => { if (event === 'token') setTokenSaved(Boolean(token())); }), []);
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
  // the days of a real export: all of them unless chosen (the sample's days are the sample's)
  const [liveRange, setLiveRange] = useState({ from: '', to: '' });
  const [format, setFormat] = useState<MessageFormat>('html');
  const [includeMedia, setIncludeMedia] = useState(true);
  // the job of the export card (#211): it runs on the server, in the task centre
  const [exportJobId, setExportJobId] = useState<string | null>(null);
  const [exportError, setExportError] = useState('');
  const [query, setQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState<string | null>(null);
  const [searchDatesOpen, setSearchDatesOpen] = useState(false);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [toast, setToast] = useState('');
  const [mobileDetail, setMobileDetail] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [theme, setTheme] = useTheme();
  // the settings: the usual ones, or the CLI's own tools and the command console (#217)
  const [settingsTab, setSettingsTab] = useState<'general' | 'advanced'>('general');
  const [consolePick, setConsolePick] = useState<string | null>(null);
  const consolePicked = useCallback(() => setConsolePick(null), []);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const consoleRef = useRef<HTMLElement>(null);
  const settingsRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  const shownPreview = usePresence(preview);
  const shownToast = usePresence(toast);

  // The chat page reads `weflow serve` once it is connected; until then, and on the other pages, sample records.
  const live = activeCategory === 'chat' && (connected || backend.state === 'checking');
  // another server or account (#198): what was read for the one before goes
  const scope = useScope(backend);
  const jobs = useJobs(connected, scope);
  // the account setup (#216): it opens by itself when a tab finds the account not set up, and from the settings
  const { profiles, failed: profilesFailed, reload: reloadProfiles } = useProfiles(connected, scope);
  const profile = profiles ? currentProfile(profiles) : undefined;
  const [setupOpen, setSetupOpen] = useState(false);
  const shownSetup = usePresence(setupOpen);
  const setupRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (profile && !profile.complete && !setupDismissed()) setSetupOpen(true);
  }, [profile?.name, profile?.complete]);
  const exportJob = findJob(jobs, exportJobId);
  const stage = stageOf(exportJob, exportError);
  const exportResult: ExportResult | null = exportJob?.status === 'succeeded' && exportJob.file
    ? { fileName: exportJob.file.name, count: 0, downloadUrl: jobFileUrl(exportJob.id) } : null;
  const [liveSessions, reloadSessions] = useSessions(connected, scope);
  const [liveId, setLiveId] = useState<string | null>(null);
  const mediaLookup = useRef<AbortController | null>(null);
  useEffect(() => () => { mediaLookup.current?.abort(); }, [scope, liveId, section]);
  const sessionList = liveSessions.status === 'ready' ? liveSessions.sessions : [];
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
  // The Moments page reads `weflow serve` too (#202): who posts, and the timeline narrowed to a poster, words, days.
  const momentsLive = activeCategory === 'moments' && connected;
  const [momentFilter, setMomentFilter] = useState<SnsFilter>(NO_FILTER);
  useEffect(() => { setMomentFilter(NO_FILTER); }, [scope]);
  const [people, reloadPeople] = useMomentPeople(momentsLive, scope, sessionList[0]?.selfWxid);
  // The WeChat Pay page reads all the notices once (#204) and narrows them by payee and days itself.
  const paymentsLive = activeCategory === 'payments' && connected;
  // The contacts page (#205): the list read once it is opened, one contact shown beside it.
  const contactsPage = section === 'contacts';
  const [contactsState, reloadContacts] = useContacts(contactsPage && connected, scope);
  const [contactKind, setContactKind] = useState<ContactKind>('friend');
  const [contactId, setContactId] = useState<string | null>(null);
  useEffect(() => { setContactId(null); }, [scope]);
  const [contactsExport, setContactsExport] = useState(DEFAULT_CONTACTS_EXPORT);
  const contacts = contactsState.status === 'ready' ? contactsState.contacts : null;
  const contact = contacts?.find(c => c.username === contactId);
  // The export page (#211): the conversations picked, and how they are exported, one job each.
  const exportPage = section === 'export';
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [batch, setBatch] = useState<BatchOptions>(DEFAULT_BATCH);
  useEffect(() => { setPicked(new Set()); }, [scope]);
  const [payFilter, setPayFilter] = useState<PayFilter>(ALL_PAYMENTS);
  useEffect(() => { setPayFilter(ALL_PAYMENTS); }, [scope]);
  const [payState, reloadPayments] = usePayments(paymentsLive, scope);
  const payDays = useMemo(() => payState.status === 'ready' ? withinDays(payState.payments, payFilter) : null, [payState, payFilter]);
  const merchants = useMemo(() => payDays ? merchantsOf(payDays) : [], [payDays]);
  const chosenPayments = useMemo(() => payDays && (payFilter.merchant === null ? payDays : payDays.filter(p => p.merchant === payFilter.merchant)), [payDays, payFilter.merchant]);
  const list = conversations.filter(item => item.category === activeCategory);
  const sessionNames = useMemo(() => new Map(sessionList.map(s => [s.username, s.displayName || s.username])), [sessionList]);
  const pickedSessions = useMemo(() => sessionList.filter(s => picked.has(s.username)), [sessionList, picked]);
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

  useEffect(() => {
    const dialog = setupRef.current;
    if (shownSetup && dialog && !dialog.open) dialog.showModal?.();
    else if (!shownSetup && dialog?.open) dialog.close();
  }, [shownSetup]);
  const closeSetup = () => { dismissSetup(); setSetupOpen(false); };

  // another conversation or other options: the card lets go of its job, which goes on in the task centre
  const resetExport = () => { setExportJobId(null); setExportError(''); };

  const selectSection = (value: Navigation) => {
    setSection(value); setMobileDetail(false);
    if (value !== 'settings') { setActiveCategory(value === 'export' || value === 'contacts' ? 'chat' : value); resetExport(); setSubmittedQuery(null); setQuery(''); }
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

  // a picture, video or file of the media card: its conversation opens at the message (#200)
  const openMedia = async (sessionId: string, item: Resource) => {
    mediaLookup.current?.abort();
    const controller = new AbortController();
    mediaLookup.current = controller;
    try {
      const message = await lookupMessage(sessionId, item.localId, item.createTime, controller.signal);
      if (controller.signal.aborted) return;
      if (!message?._cursor) { setToast('没有找到这条消息。'); return; }
      setJumpTo({ sessionId, target: { at: 'message', message }, seq: ++jumps.current });
      showMessages();
    } catch {
      if (!controller.signal.aborted) setToast('没有找到这条消息。');
    }
  };

  const jumpToDay = (sessionId: string, day: string) => {
    setJumpTo({ sessionId, target: { at: 'time', seconds: parseDay(day).getTime() / 1000 }, seq: ++jumps.current });
    showMessages();
  };

  const exportRecords = async () => {
    if (!liveSession) return;
    setExportError(''); setExportJobId(null);
    try {
      const job = await runJob('导出 ' + selected.name + ' · ' + formatLabel(format), 'POST', 'export/messages', {
        'session-id': liveSession.username, format, start: liveRange.from || undefined, end: liveRange.to || undefined,
        media: includeMedia ? 'all' : undefined,
      });
      setExportJobId(job.id);
    } catch (problem) {
      setExportError(problem instanceof Error && problem.message ? problem.message : '导出没有开始，请重试。');
    }
  };
  const cancelExport = () => {
    if (exportJobId) removeJob(exportJobId).catch(() => setExportError('取消失败，请重试。'));
  };

  const search = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSubmittedQuery(query.trim());
  };

  const cards: Card[] = [
    {
      id: 'export', title: '导出', summary: formatLabel(format) + (includeMedia ? ' · 含媒体' : ''),
      content: <>
        <p className="card-lead"><span className="clamp-1">{selected.name}的{selected.category === 'chat' ? '聊天记录' : '记录'}</span>{!selected.live && <span className="badge">示例数据</span>}</p>
        <div className="form-row"><span className="field-label">时间范围</span>{selected.live
          ? <DateRange label="导出的时间范围" from={liveRange.from} to={liveRange.to} onChange={range => { setLiveRange(range); resetExport(); }} />
          : <DateRange label="导出的时间范围" from={from} to={to} onChange={range => { setFrom(range.from); setTo(range.to); resetExport(); }} />}</div>
        <div className="form-row format-row"><span className="field-label">导出格式</span>
          <div className="choice-chips" role="group" aria-label="导出格式">
            {MESSAGE_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={format === f.value} onClick={() => { setFormat(f.value); resetExport(); }}>{f.label}</button>)}
          </div>
          <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={includeMedia} onChange={event => { setIncludeMedia(event.target.checked); resetExport(); }} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>附带媒体文件 <small>图片、语音、视频与表情</small></span></label>
        </div>
        {selected.live && liveSession
          ? <><ExportControl stage={stage} result={exportResult} onStart={exportRecords} onCancel={cancelExport} />
            {stage === 'error' && <p className="card-note" role="alert">{exportJob ? jobDetail(exportJob) : exportError}</p>}
            {stage === 'exporting' && exportJob && <p className="card-note" aria-live="polite">{exportJob.status === 'queued' ? '排队中：前面的任务完成后开始。' : jobDetail(exportJob)}</p>}
            {stage === 'done' && includeMedia && outputOf(exportJob!) && <p className="card-note">媒体文件留在服务端，位于 {outputOf(exportJob!)} 旁的 media 文件夹；在任务中心删除任务时，导出文件和媒体文件一并删除。</p>}</>
          : <><button type="button" className="primary-button large export-pending" disabled><DownloadSimple size={18} />开始导出</button><p className="card-note">{selected.live ? '选择一个会话后导出。' : '连接 weflow serve 后，可以导出本机的真实记录。'}</p></>}
      </>,
    },
    {
      id: 'tasks', title: '任务中心',
      summary: connected ? jobsSummary(jobs) : '未连接',
      content: connected ? <TaskCenter jobs={jobs} names={sessionNames} /> : <p className="empty-text">连接 weflow serve 后，导出等任务会显示在这里。</p>,
    },
    {
      id: 'search', title: '搜索', summary: selected.live ? searchSummary(liveSearch.state) : submittedQuery === null ? '当前会话' : results.length + ' 条结果',
      content: selected.live
        ? <SearchPanel key={scope} session={liveSession} sessions={sessionList} search={liveSearch} onOpenHit={openHit} onJumpDay={jumpToDay} />
        : <>
        <form className="search-form" onSubmit={search}>
          <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" /><input type="search" aria-label="搜索此会话的消息" placeholder={'搜索' + selected.name + '的消息'} value={query} onChange={event => setQuery(event.target.value)} /></div>
          <button className="primary-button search-button" type="submit" aria-label="搜索"><MagnifyingGlass size={16} /><span className="search-button-label">搜索</span></button>
          <button className={'icon-button' + (searchDatesOpen ? ' on' : '')} type="button" aria-label="筛选搜索日期" aria-expanded={searchDatesOpen} onClick={() => setSearchDatesOpen(value => !value)}><CalendarBlank size={16} /></button>
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
        : <div className="media-list"><HoverTrack items=".media-item" />{media.length ? media.map(message => <button className="media-item" type="button" key={message.id} onClick={() => message.asset ? setPreview({ type: 'image', src: message.asset, title: message.text }) : setPreview({ type: 'file', title: message.fileName || message.text })}>{message.asset ? <img src={message.asset} alt="" /> : <FilePdf size={32} />}<span><span className="clamp-1">{message.fileName || message.text}</span><small>{message.date} {message.time}</small></span><ArrowSquareOut size={14} /></button>) : <p className="empty-text">这段记录中没有媒体文件。</p>}</div>,
    },
    {
      id: 'moments-filter', title: '筛选朋友圈', summary: filterSummary(momentFilter),
      content: <MomentsFilter key={scope} filter={momentFilter} onChange={setMomentFilter} />,
    },
    {
      id: 'moments-stats', title: '朋友圈统计',
      content: (open: boolean) => <MomentsStats scope={scope} active={open && momentsLive} />,
    },
    {
      id: 'pay-filter', title: '筛选支付记录', summary: payFilterSummary(payFilter),
      content: <PaymentsFilter filter={payFilter} onChange={setPayFilter} />,
    },
    {
      id: 'pay-summary', title: '合计', summary: chosenPayments ? chosenPayments.length + ' 笔' : '',
      content: <PaymentsSummary payments={chosenPayments} />,
    },
    {
      id: 'pay-export', title: '导出支付记录',
      content: <PaymentsExport key={scope + ':' + JSON.stringify(payFilter)} payments={chosenPayments} filter={payFilter} />,
    },
    {
      id: 'batch-export', title: '批量导出', summary: batchSummary(pickedSessions.length, batch),
      content: <BatchExport picked={pickedSessions} options={batch} onChange={setBatch} />,
    },
    { id: 'sessions-export', title: '导出会话列表', content: <SessionsExport picked={pickedSessions} /> },
    {
      id: 'contacts-summary', title: '通讯录统计', summary: contacts ? contacts.length.toLocaleString('zh-CN') + ' 个' : '',
      content: <ContactsSummary contacts={contacts} />,
    },
    {
      id: 'contacts-export', title: '导出通讯录', summary: contactsExportSummary(contactsExport),
      content: <ContactsExport options={contactsExport} onChange={setContactsExport} ready={Boolean(contacts)} />,
    },
  ];
  // nothing to show of a real conversation until one is open; the Moments and WeChat Pay pages have cards of their own
  const chatCards = ['export', 'tasks', 'search', 'info', 'media'];
  const momentCards = ['moments-filter', 'moments-stats'];
  const payCards = ['pay-filter', 'pay-summary', 'pay-export'];
  const exportCards = ['batch-export', 'sessions-export'];
  const contactCards = ['contacts-summary', 'contacts-export'];
  const hiddenCards = exportPage
    ? [...chatCards.filter(id => id !== 'tasks'), ...momentCards, ...payCards, ...contactCards, ...(connected ? [] : exportCards)]
    : contactsPage
      ? [...chatCards.filter(id => id !== 'tasks'), ...momentCards, ...payCards, ...exportCards, ...(connected ? [] : contactCards)]
      : momentsLive
        ? [...chatCards, ...payCards, ...exportCards, ...contactCards]
        : paymentsLive
          ? [...chatCards, ...momentCards, ...exportCards, ...contactCards]
          : [...momentCards, ...payCards, ...exportCards, ...contactCards, ...(selected.live && !liveSession ? ['info', 'media'] : [])];

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
        <div className="settings-heading"><Gear size={20} /><h1>设置</h1>
          <Segmented className="settings-tabs" count={2} index={settingsTab === 'general' ? 0 : 1} role="group" aria-label="设置分组">
            <button type="button" aria-pressed={settingsTab === 'general'} onClick={() => setSettingsTab('general')}>常规</button>
            <button type="button" aria-pressed={settingsTab === 'advanced'} onClick={() => setSettingsTab('advanced')}>高级</button>
          </Segmented></div>
        {settingsTab === 'advanced' ? <>
          <p className="muted">CLI 独有的工具，以及按 OpenAPI 运行任意命令的控制台。</p>
          {connected
            ? <><AdvancedTools onOpen={setConsolePick} /><h2 className="settings-subheading">命令控制台</h2><CommandConsole pick={consolePick} onPicked={consolePicked} /></>
            : <p className="settings-footnote">连接 weflow serve 后才能使用。</p>}
        </> : <>
        <p className="muted">管理连接和浏览偏好。</p>
        <div className="setting-row"><div><h2>weflow serve</h2><p>{backendDetail(backend)}</p></div><button className="secondary-button" type="button" onClick={recheckBackend}>重新检测</button></div>
        {connected && (profiles
          ? <div className="setting-row setting-accounts"><div><h2>账号</h2><p>每个配置档案对应一个微信账号。切换档案后，各页重新读取这个账号的记录。</p>
            <Accounts profiles={profiles} onChanged={reloadProfiles} onSetup={() => setSetupOpen(true)} /></div></div>
          : profilesFailed
            ? <div className="setting-row"><div><h2>账号</h2><p>读不到账号配置：weflow 5.0.0-rc.7 起才能在网页里查看。</p></div>
              <button className="secondary-button" type="button" onClick={() => setSetupOpen(true)}>配置向导</button></div>
            : <div className="setting-row setting-accounts"><div><h2>账号</h2><p>每个配置档案对应一个微信账号。切换档案后，各页重新读取这个账号的记录。</p><AccountsSkeleton /></div></div>)}
        {'health' in backend && <div className="setting-row"><div><h2>weflow 版本</h2>
          <p>weflow {backend.health.version} · API {backend.health.apiVersion}</p>
          {backend.state === 'incompatible' && <p className="setting-warning">{backend.reason}。{backend.advice}</p>}
        </div><span className={'source-name' + (backend.state === 'incompatible' ? ' warning' : '')}>{backend.state === 'incompatible' ? '不适用于这个网页' : '适用于这个网页'}</span></div>}
        <div className="setting-row"><div><h2>网页版本</h2><p>需要 API {WEB.apiVersion}，weflow {WEB.minBackendVersion} 或更新的版本。</p></div><span className="source-name"><Plugs size={14} />{WEB.version}</span></div>
        <div className="setting-row setting-token"><div><h2>令牌</h2>
          <p>{tokenSaved ? '这个标签页保存了令牌，关闭后清除。' : '没有保存令牌：从 weflow serve 启动时给出的地址打开网页，可自动连接。'}需要更换时在这里粘贴，只保存在这个标签页里。</p>
          <TokenForm label="新的令牌" />
        </div>{tokenSaved && <button className="secondary-button" type="button" onClick={() => setToken(null)}>清除</button>}</div>
        {'health' in backend && serverDetail(backend.health) && <div className="setting-row"><div><h2>运行方式</h2><p>{serverDetail(backend.health)}</p></div></div>}
        {connected && <><LanguageSetting /><AiSetting profile={profile} onSaved={reloadProfiles} /><CacheSetting /></>}
        <div className="setting-row"><div><h2>实时消息</h2><p>微信收到消息后几秒内显示在会话列表和打开的会话里。关掉后不再接收，左下角显示「已连接」而不是「实时」。</p></div><label className="toggle"><input type="checkbox" aria-label="实时消息" checked={realtime} onChange={event => toggleRealtime(event.target.checked)} /><span /></label></div>
        <div className="setting-row"><div><h2>默认附带媒体文件</h2><p>导出时同时保存照片等媒体。</p></div><label className="toggle"><input type="checkbox" aria-label="默认附带媒体文件" checked={includeMedia} onChange={event => setIncludeMedia(event.target.checked)} /><span /></label></div>
        <div className="setting-row setting-choice"><div><h2>颜色主题</h2><p>跟随系统时随系统的浅色、深色模式切换。只保存在这个浏览器里。</p></div>
          <Segmented className="theme-group" count={THEMES.length} index={THEMES.findIndex(t => t.value === theme)} role="group" aria-label="颜色主题">
            {THEMES.map(t => <button type="button" key={t.value} aria-pressed={theme === t.value} onClick={() => setTheme(t.value)}>{t.label}</button>)}
          </Segmented></div>
        <div className="setting-row"><div><h2>减少动画</h2><p>所有过渡立即完成，状态提示保留。也跟随系统的「减少动态效果」。</p></div><label className="toggle"><input type="checkbox" aria-label="减少动画" checked={reducedMotion} onChange={event => setReducedMotion(event.target.checked)} /><span /></label></div>
        <small className="settings-footnote">连接到 weflow serve 后读取本机记录；未连接时显示示例数据。</small>
        </>}
      </section><Scrollbar target={settingsRef} /></> : momentsLive || (activeCategory === 'moments' && backend.state === 'unauthorized')
        ? <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label="朋友圈浏览">
          {backend.state === 'unauthorized'
            ? <TokenPrompt />
            : <MomentPeople state={people} total={null} selected={momentFilter.username} onRetry={reloadPeople}
              onSelect={username => { setMomentFilter(filter => ({ ...filter, username })); setMobileDetail(true); }} />}
          <div ref={paneRef} className="message-pane scroll-area" aria-label="朋友圈时间线">
            <button type="button" className="mobile-back icon-button" aria-label="返回发布者列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
            {momentsLive && <MomentsFeed key={scope + ':' + JSON.stringify(momentFilter)} filter={momentFilter} scroller={paneRef} onPreview={setPreview} />}
          </div>
          <Scrollbar target={paneRef} />
        </section>
        : contactsPage
        ? <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label="通讯录">
          {backend.state === 'unauthorized'
            ? <TokenPrompt />
            : connected
              ? <ContactList state={contactsState} kind={contactKind} onKind={setContactKind} selected={contactId} onRetry={reloadContacts}
                onSelect={username => { setContactId(username); setMobileDetail(true); }} />
              : <div className="session-pane"><p className="session-message">连接 weflow serve 后，可以在这里查看通讯录。</p></div>}
          <div ref={paneRef} className="message-pane scroll-area" aria-label="联系人资料">
            <button type="button" className="mobile-back icon-button" aria-label="返回联系人列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
            {connected && (contact
              ? <ContactDetail key={contact.username} contact={contact} canChat={sessionList.some(s => s.username === contact.username)}
                onChat={() => { selectSection('chat'); setLiveId(contact.username); setMobileDetail(true); }} />
              : <div className="pane-message">{contacts && <p>选择左侧的一个联系人。</p>}</div>)}
          </div>
          <Scrollbar target={paneRef} />
        </section>
        : exportPage
        ? <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label="导出">
          {backend.state === 'unauthorized'
            ? <TokenPrompt />
            : connected
              ? <ExportPicker state={liveSessions} picked={picked} onChange={setPicked} onRetry={reloadSessions} />
              : <div className="session-pane"><p className="session-message">连接 weflow serve 后，可以在这里选择会话批量导出。</p></div>}
          <div ref={paneRef} className="message-pane scroll-area" aria-label="已选的会话">
            <button type="button" className="mobile-back icon-button" aria-label="返回会话列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
            {connected && <PickedList picked={pickedSessions} onRemove={id => setPicked(old => { const next = new Set(old); next.delete(id); return next; })} />}
          </div>
          <Scrollbar target={paneRef} />
        </section>
        : paymentsLive || (activeCategory === 'payments' && backend.state === 'unauthorized')
        ? <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label="微信支付浏览">
          {backend.state === 'unauthorized'
            ? <TokenPrompt />
            : <PaymentMerchants state={payState} merchants={merchants} selected={payFilter.merchant} onRetry={reloadPayments}
              onSelect={merchant => { setPayFilter(filter => ({ ...filter, merchant })); setMobileDetail(true); }} />}
          <div ref={paneRef} className="message-pane scroll-area" aria-label="支付记录">
            <button type="button" className="mobile-back icon-button" aria-label="返回商户列表" onClick={() => setMobileDetail(false)}><ArrowLeft size={18} /></button>
            {paymentsLive && (chosenPayments
              ? <PaymentList key={scope + ':' + JSON.stringify(payFilter)} payments={chosenPayments} scroller={paneRef} narrowed={Boolean(payFilter.merchant || payFilter.from || payFilter.to)} />
              : payState.status === 'loading' && <PaymentsSkeleton label="正在读取支付记录" />)}
          </div>
          <Scrollbar target={paneRef} />
        </section>
        : <section className={'panel browser-panel' + (mobileDetail ? ' show-detail' : '')} aria-label={categoryLabels[activeCategory] + '记录浏览'}>
        {activeCategory === 'chat'
          ? backend.state === 'unauthorized'
            ? <TokenPrompt />
            : <SessionList state={connected ? liveSessions : backend.state === 'checking' ? CHECKING : SAMPLE_SESSIONS} sample={!connected && backend.state !== 'checking'} selectedId={selected.id || null} onSelect={selectConversation} onRetry={reloadSessions}
              officials={officials} onFilter={filter => { if (filter === 'official') setOfficialsWanted(true); }} />
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
            ? kindOf(liveSession.username) === 'official'
              ? <BizPushes key={scope + ':' + liveSession.username} session={liveSession} />
              : <LiveMessages key={scope} session={liveSession} scroller={paneRef} jump={jumpTo} onPreview={setPreview} />
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
    <dialog className="setup-dialog" ref={setupRef} aria-label="账号配置" data-state={shownSetup?.closing ? 'closing' : 'open'}
      onCancel={event => { event.preventDefault(); closeSetup(); }}>
      {shownSetup && <SetupWizard profile={profile} jobs={jobs} onChanged={reloadProfiles} onClose={closeSetup}
        onDone={() => { setSetupOpen(false); selectSection('chat'); }} />}
    </dialog>
    {shownToast && <div className="toast" role="status" data-state={shownToast.closing ? 'closing' : 'open'}><CheckCircle size={16} />{shownToast.value}</div>}
  </main>;
}
