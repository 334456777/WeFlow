// 工作区（#130）：左边是导航，中间是页面，右边是操作控制台。
// 这里保存各页面共用的状态（服务端、账号、会话列表、作业），从一个页面打开另一个页面，
// 并把各页面的卡片收进同一个操作控制台；每个页面保存自己的状态（#401）。
// 连上 `weflow serve` 后记录来自它；在那之前由 src/mock 的示例服务端代替它回答各页面（#879），
// 作业和设置也一样（#896）。连上过的服务端断开后，已经从它读到的记录继续显示，直到它恢复。
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
import { useSessions } from './useSessions';
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

/** 页面浏览的是哪一类记录：聊天、朋友圈或微信支付。 */
const categoryOf = (section: Navigation): Category => section === 'moments' || section === 'payments' ? section : 'chat';

/** 示例服务端，单独的 chunk：只有没有服务端回答的页面才下载它。 */
const loadSample = (): Promise<SampleServer> => import('./mock/server').then(module => module.server);

/** 整个页面：导航、打开的页面和操作控制台，以及它们共用的状态。 */
export function App() {
  // 键盘操作（#195）：每组控件只占一个 Tab 停靠点，组内用方向键移动；按 / 搜索
  useRovingFocus();
  useSearchKey();
  const prefs = usePreferences();
  const [backend, recheckBackend] = useBackend(prefs.realtime);
  // 服务端告知的语言（#439）：语言变化时用它重新画出各页面
  const language = useLang();
  const connected = backend.state === 'ready';
  const held = holdsRecords(backend);
  const sample = recordsOf(backend) === 'sample';
  const records = held || sample;
  // 示例服务端代替服务端回答页面的读取（#879）：在发出任何读取之前设置好
  useLayoutEffect(() => {
    if (!sample) return;
    setSample(loadSample);
    return () => setSample(null);
  }, [sample]);
  // 作业和设置发给回答的服务端，或者代替它的示例服务端（#896）
  const usable = connected || sample;
  const health = useActingHealth(backend, sample);
  const fold = useSidebarFold();
  const [section, setSection] = useState<Navigation>('chat');
  // 最后浏览的那一类记录：打开设置时保留它，操作控制台里仍显示它的卡片
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

  // 换了服务端或账号（#198）：为之前那一个读到的内容都清掉
  const scope = useScope(backend);
  const jobs = useJobs(usable, scope);
  // 配置向导（#216）：标签页发现账号还没配置好时自动打开（关掉过就不再自动打开），也可以从设置里打开
  const profiles = useProfiles(usable, scope, jobs.jobs);
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
  // 从其他页面跳到聊天页：打开一个会话，或者定位到一条消息（消息所在的会话不在会话列表里时提示 `missing`）
  const chatWith = (username: string) => { selectSection('chat'); chat.open(username); setMobileDetail(true); };
  const openInChat = (missing: string): OpenAt => (sessionId, localId, createTime) => {
    if (!sessionList.some(session => session.username === sessionId)) { setToast(missing); return; }
    selectSection('chat');
    chat.openAt(sessionId, localId, createTime);
  };
  const settings = useSettingsPage(shell, prefs, { profiles, recheck: recheckBackend, onSetup: () => setSetupOpen(true) });
  // 从灵感信箱页打开 AI 设置，在那里选择为哪些会话写见解（#1078）
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
  // 还没有内容可显示的页面（还在找服务端时的朋友圈、微信支付）沿用聊天页的视图和卡片
  const page = pages[section];
  const tasks: Card = {
    id: 'tasks', title: t('任务中心'),
    summary: usable ? jobsSummary(jobs) : held ? t('已断开') : t('未连接'),
    content: usable ? <TaskCenter jobs={jobs} names={sessionNames} /> : <p className="empty-text">{t('连接 weflow serve 后，导出等任务会显示在这里。')}</p>,
  };
  // 全部卡片按最初的顺序排列，任务中心排在当前会话的导出卡片后面；操作控制台保存每位用户自己调整的顺序，
  // 显示当前页面的卡片，打开设置时显示之前浏览的那一类记录的卡片
  const [exportCard, ...chatCards] = chat.cards;
  const cards = [exportCard, tasks, ...chatCards, ...Object.values(pages).filter(other => other !== chat).flatMap(other => other.cards ?? [])];
  const shownCards = (section === 'settings' ? pages[category] : page).shown ?? chat.shown;
  const hiddenCards = cards.map(card => card.id).filter(id => !shownCards.includes(id));

  // 换页时新的页面像内容一样进入（#776），或者按皮肤的方式进入（`pageIn`，#832）；
  // 两个页面的面板形状相同时，面板元素保留不变。操作控制台换了显示的卡片时也一起进入，
  // 进入设置时不换（设置保留之前浏览的那一类记录的卡片）
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
