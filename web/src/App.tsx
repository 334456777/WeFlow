// The workspace (#130): navigation on the left, a page in the middle, the operations console on the right. It holds
// what the pages share (the server, the account, the conversation list, the jobs), opens one page
// from another and gathers the pages' cards into the one console; each page keeps its own state (#401).
// Records come from `weflow serve` once connected; until then the sample server of src/mock answers the pages in its
// place (#879), jobs and settings too (#896). When the server goes after it was connected, what was read from it stays
// shown until it is back.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CheckCircle } from '@phosphor-icons/react';
import { useAnalyticsPage } from './analytics/useAnalyticsPage';
import { useBackend } from './api/backend';
import { holdsRecords, recordsOf } from './api/records';
import { setSample, type SampleServer } from './api/client';
import { t, useLang } from './i18n';
import { useScope } from './api/scope';
import { currentProfile } from './api/setup';
import { backupPage } from './backup/BackupPage';
import { CardStack, type Card } from './CardStack';
import { useChatPage } from './chat/useChatPage';
import { useSessions } from './chat/useSessions';
import { useContactsPage } from './contacts/useContactsPage';
import { useExportPage } from './exports/useExportPage';
import { useFootprintPage } from './footprint/useFootprintPage';
import { useInsightsPage } from './insights/useInsightsPage';
import { jobsSummary, TaskCenter } from './jobs/TaskCenter';
import { useJobs } from './jobs/useJobs';
import { useMomentsPage } from './moments/useMomentsPage';
import { FEEDBACK_MS, pageIn, usePresence } from './motion';
import { usePaymentsPage } from './payments/usePaymentsPage';
import { useReportsPage } from './reports/useReportsPage';
import { useResourcesPage } from './resources/useResourcesPage';
import { useRovingFocus, useSearchKey } from './roving';
import { Scrollbar } from './Scrollbar';
import { SetupDialog } from './settings/SetupDialog';
import { dismissSetup, setupDismissed, useProfiles } from './settings/useProfiles';
import { useSettingsPage } from './settings/useSettingsPage';
import { useActingHealth } from './shell/acting';
import type { OpenAt, Page, Shell } from './shell/page';
import { usePreferences } from './shell/preferences';
import { usePreview } from './shell/PreviewDialog';
import { SampleContext } from './shell/sample';
import { Sidebar, useSidebarFold } from './shell/Sidebar';
import type { Category, Navigation } from './types';

/** The records a page browses: the chat's, the Moments' or WeChat Pay's. */
const categoryOf = (section: Navigation): Category => section === 'moments' || section === 'payments' ? section : 'chat';

/** The sample server, a chunk of its own: downloaded only by a page that no server answers. */
const loadSample = (): Promise<SampleServer> => import('./mock/server').then(module => module.server);

/** The whole page: the navigation, the open page and the console, with the state they share. */
export function App() {
  // the keyboard (#195): one Tab stop per group, arrows within it; / to search
  useRovingFocus();
  useSearchKey();
  const prefs = usePreferences();
  const [backend, recheckBackend] = useBackend(prefs.realtime);
  // the server's language, once it said (#439): the pages are drawn again in it
  const language = useLang();
  const connected = backend.state === 'ready';
  const held = holdsRecords(backend);
  const sample = recordsOf(backend) === 'sample';
  const records = held || sample;
  // the sample server answers the page's reads in its place (#879): set before any of them is made
  useLayoutEffect(() => {
    if (!sample) return;
    setSample(loadSample);
    return () => setSample(null);
  }, [sample]);
  // jobs and settings go to the server that answers, or to the sample server in its place (#896)
  const usable = connected || sample;
  const health = useActingHealth(backend, sample);
  const fold = useSidebarFold();
  const [section, setSection] = useState<Navigation>('chat');
  // the records browsed last, which the settings keep, with their cards in the console
  const [category, setCategory] = useState<Category>('chat');
  const [mobileDetail, setMobileDetail] = useState(false);
  const [toast, setToast] = useState('');
  const shownToast = usePresence(toast);
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(''), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [toast]);
  const { open: openPreview, dialog: previewDialog } = usePreview(prefs.reducedMotion);
  const consoleRef = useRef<HTMLElement>(null);

  // another server or account (#198): what was read for the one before goes
  const scope = useScope(backend);
  const jobs = useJobs(usable, scope);
  // the account setup (#216): it opens by itself when a tab finds the account not set up, and from the settings
  const profiles = useProfiles(usable, scope);
  const profile = profiles.profiles ? currentProfile(profiles.profiles) : undefined;
  const [setupOpen, setSetupOpen] = useState(false);
  useEffect(() => {
    if (profile && !profile.complete && !setupDismissed()) setSetupOpen(true);
  }, [profile?.name, profile?.complete]);
  const closeSetup = () => { dismissSetup(); setSetupOpen(false); };
  const [sessions, reloadSessions] = useSessions(records, scope);
  const sessionList = sessions.status === 'ready' ? sessions.sessions : [];
  const sessionNames = useMemo(() => new Map(sessionList.map(s => [s.username, s.displayName || s.username])), [sessionList]);
  const shell: Shell = { backend, connected, usable, health, held, records, sample, scope, sessions, sessionList, reloadSessions, jobs, profile, mobileDetail, setMobileDetail, openPreview, toast: setToast };

  const chat = useChatPage(shell, prefs);
  const selectSection = (value: Navigation) => {
    chat.leave(value !== 'settings');
    setSection(value); setMobileDetail(false);
    if (value !== 'settings') setCategory(categoryOf(value));
  };
  // from another page: the chat page opens at a conversation, or at a message when its conversation is in the list
  const chatWith = (username: string) => { selectSection('chat'); chat.open(username); setMobileDetail(true); };
  const openInChat = (missing: string): OpenAt => (sessionId, localId, createTime) => {
    if (!sessionList.some(session => session.username === sessionId)) { setToast(missing); return; }
    selectSection('chat');
    chat.openAt(sessionId, localId, createTime);
  };
  const settings = useSettingsPage(shell, prefs, { profiles, recheck: recheckBackend, onSetup: () => setSetupOpen(true) });
  // from the insights page: the AI settings, where the conversations the insights are written for are chosen (#1078)
  const openAiSettings = () => { selectSection('settings'); settings.openSection('ai'); };
  const pages: Record<Navigation, Page> = {
    chat,
    moments: useMomentsPage(shell, category === 'moments'),
    payments: usePaymentsPage(shell, category === 'payments'),
    contacts: useContactsPage(shell, section === 'contacts', chatWith),
    resources: useResourcesPage(shell, openInChat(t('这条消息所在的会话不在会话列表里。'))),
    analytics: useAnalyticsPage(shell, section === 'analytics', chatWith),
    reports: useReportsPage(shell),
    footprint: useFootprintPage(shell, openInChat(t('这段聊天所在的会话不在会话列表里。'))),
    insights: useInsightsPage(shell, section === 'insights', chatWith, openAiSettings),
    export: useExportPage(shell),
    backup: backupPage(shell),
    settings,
  };
  // a page with nothing to show yet (Moments, WeChat Pay while the server is looked for) leaves the chat page's
  const page = pages[section];
  const tasks: Card = {
    id: 'tasks', title: t('任务中心'),
    summary: usable ? jobsSummary(jobs) : held ? t('已断开') : t('未连接'),
    content: usable ? <TaskCenter jobs={jobs} names={sessionNames} /> : <p className="empty-text">{t('连接 weflow serve 后，导出等任务会显示在这里。')}</p>,
  };
  // every card in its first order, the task centre after the open conversation's export; the console keeps each
  // viewer's own order and shows those of the page, or of the records browsed while the settings are open
  const [exportCard, ...chatCards] = chat.cards;
  const cards = [exportCard, tasks, ...chatCards, ...Object.values(pages).filter(other => other !== chat).flatMap(other => other.cards ?? [])];
  const shownCards = (section === 'settings' ? pages[category] : page).shown ?? chat.shown;
  const hiddenCards = cards.map(card => card.id).filter(id => !shownCards.includes(id));

  // another page comes in as content does (#776) or as the skin brings it (pageIn, #832), its panel kept when two pages
  // share its shape; the console with it
  // when it shows other cards (not into the settings, which keep those of the records browsed before)
  const mainRef = useRef<HTMLDivElement>(null);
  const shownCardIds = shownCards.join(' ');
  const shownPage = useRef({ section, cards: shownCardIds });
  useEffect(() => {
    const before = shownPage.current;
    shownPage.current = { section, cards: shownCardIds };
    if (before.section === section) return;
    const panel = mainRef.current?.querySelector(':scope > .panel');
    pageIn([panel, before.cards !== shownCardIds && consoleRef.current].filter((element): element is Element => Boolean(element)));
  }, [section, shownCardIds]);

  return <SampleContext.Provider value={sample}><main key={language} className={'workspace' + (fold.folded ? ' sidebar-collapsed' : '')}>
    <Sidebar section={section} onSelect={selectSection} fold={fold} backend={backend} realtime={prefs.realtime} />
    <div ref={mainRef} className="main-area">
      {page.view ?? chat.view}
      <section ref={consoleRef} className="console scroll-area" aria-label={t('操作控制台')}>
        <CardStack cards={cards} hidden={hiddenCards} storageKey="weflow.console" defaultClosed={['search', 'info', 'media']} label={t('操作控制台')} />
      </section>
      <Scrollbar target={consoleRef} />
    </div>
    {previewDialog}
    <SetupDialog open={setupOpen} profile={profile} jobs={jobs} platform={health?.platform}
      onChanged={profiles.reload} onClose={closeSetup} onDone={() => { setSetupOpen(false); selectSection('chat'); }} />
    {shownToast && <div className="toast" role="status" data-state={shownToast.closing ? 'closing' : 'open'}><CheckCircle size={16} />{shownToast.value}</div>}
  </main></SampleContext.Provider>;
}
