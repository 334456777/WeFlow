// The workspace (#130): navigation on the left, a page in the middle, the operations console on the right as in
// Claude.ai. It holds what the pages share (the server, the account, the conversation list, the jobs), opens one page
// from another and gathers the pages' cards into the one console; each page keeps its own state (#401).
// Records come from `weflow serve` once connected; until then the lists and the console show the sample (src/mock).
import { useEffect, useMemo, useRef, useState } from 'react';
import { CheckCircle } from '@phosphor-icons/react';
import { useAnalyticsPage } from './analytics/useAnalyticsPage';
import { useBackend } from './api/backend';
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
import { FEEDBACK_MS, usePresence } from './motion';
import { usePaymentsPage } from './payments/usePaymentsPage';
import { useReportsPage } from './reports/useReportsPage';
import { useResourcesPage } from './resources/useResourcesPage';
import { useRovingFocus, useSearchKey } from './roving';
import { Scrollbar } from './Scrollbar';
import { SetupDialog } from './settings/SetupDialog';
import { dismissSetup, setupDismissed, useProfiles } from './settings/useProfiles';
import { useSettingsPage } from './settings/useSettingsPage';
import type { OpenAt, Page, Shell } from './shell/page';
import { usePreferences } from './shell/preferences';
import { usePreview } from './shell/PreviewDialog';
import { Sidebar, useSidebarFold } from './shell/Sidebar';
import type { Category, Navigation } from './types';

/** The records a page browses: the chat's, or the sample's of the Moments and WeChat Pay pages. */
const categoryOf = (section: Navigation): Category => section === 'moments' || section === 'payments' ? section : 'chat';

export function App() {
  // the keyboard (#195): one Tab stop per group, arrows within it; / to search
  useRovingFocus();
  useSearchKey();
  const prefs = usePreferences();
  const [backend, recheckBackend] = useBackend(prefs.realtime);
  const connected = backend.state === 'ready';
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
  const jobs = useJobs(connected, scope);
  // the account setup (#216): it opens by itself when a tab finds the account not set up, and from the settings
  const profiles = useProfiles(connected, scope);
  const profile = profiles.profiles ? currentProfile(profiles.profiles) : undefined;
  const [setupOpen, setSetupOpen] = useState(false);
  useEffect(() => {
    if (profile && !profile.complete && !setupDismissed()) setSetupOpen(true);
  }, [profile?.name, profile?.complete]);
  const closeSetup = () => { dismissSetup(); setSetupOpen(false); };
  const [sessions, reloadSessions] = useSessions(connected, scope);
  const sessionList = sessions.status === 'ready' ? sessions.sessions : [];
  const sessionNames = useMemo(() => new Map(sessionList.map(s => [s.username, s.displayName || s.username])), [sessionList]);
  const shell: Shell = { backend, connected, scope, sessions, sessionList, reloadSessions, jobs, profile, mobileDetail, setMobileDetail, openPreview, toast: setToast };

  const chat = useChatPage(shell, prefs, category);
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
  const pages: Record<Navigation, Page> = {
    chat,
    moments: useMomentsPage(shell, category === 'moments'),
    payments: usePaymentsPage(shell, category === 'payments'),
    contacts: useContactsPage(shell, section === 'contacts', chatWith),
    resources: useResourcesPage(shell, openInChat('这条消息所在的会话不在会话列表里。')),
    analytics: useAnalyticsPage(shell, section === 'analytics', chatWith),
    reports: useReportsPage(shell),
    footprint: useFootprintPage(shell, openInChat('这段聊天所在的会话不在会话列表里。')),
    insights: useInsightsPage(shell, section === 'insights', chatWith),
    export: useExportPage(shell),
    backup: backupPage(shell),
    settings: useSettingsPage(shell, prefs, { profiles, recheck: recheckBackend, onSetup: () => setSetupOpen(true) }),
  };
  // a page not connected to show (Moments, WeChat Pay) shows the sample in the chat page's browser
  const page = pages[section];
  const tasks: Card = {
    id: 'tasks', title: '任务中心',
    summary: connected ? jobsSummary(jobs) : '未连接',
    content: connected ? <TaskCenter jobs={jobs} names={sessionNames} /> : <p className="empty-text">连接 weflow serve 后，导出等任务会显示在这里。</p>,
  };
  // every card in its first order, the task centre after the open conversation's export; the console keeps each
  // viewer's own order and shows those of the page, or of the records browsed while the settings are open
  const [exportCard, ...chatCards] = chat.cards;
  const cards = [exportCard, tasks, ...chatCards, ...Object.values(pages).filter(other => other !== chat).flatMap(other => other.cards ?? [])];
  const shownCards = (section === 'settings' ? pages[category] : page).shown ?? chat.shown;
  const hiddenCards = cards.map(card => card.id).filter(id => !shownCards.includes(id));

  return <main className={'workspace' + (fold.folded ? ' sidebar-collapsed' : '')}>
    <Sidebar section={section} onSelect={selectSection} fold={fold} backend={backend} realtime={prefs.realtime} />
    <div className="main-area">
      {page.view ?? chat.view}
      <section ref={consoleRef} className="console scroll-area" aria-label="操作控制台">
        <CardStack cards={cards} hidden={hiddenCards} storageKey="weflow.console" defaultClosed={['search', 'info', 'media']} label="操作控制台" />
      </section>
      <Scrollbar target={consoleRef} />
    </div>
    {previewDialog}
    <SetupDialog open={setupOpen} profile={profile} jobs={jobs} onChanged={profiles.reload} onClose={closeSetup}
      onDone={() => { setSetupOpen(false); selectSection('chat'); }} />
    {shownToast && <div className="toast" role="status" data-state={shownToast.closing ? 'closing' : 'open'}><CheckCircle size={16} />{shownToast.value}</div>}
  </main>;
}
