// The settings (#214, #776): a list of the sections, by where what they hold is kept, beside the section open (on a
// phone, one at a time). A section is groups of rows: the connection and the token, the accounts, what this browser
// keeps, how the server starts, the AI model, the cache, the page's versions, and the CLI's own tools with the command
// console (#217).
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { WEB } from '../api/backend';
import { backendLabel } from '../shell/Connection';
import type { Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import type { Preferences } from '../shell/preferences';
import { useTheme } from '../theme';
import { Accounts, AccountsSkeleton, profileState } from './Accounts';
import { AdvancedTools } from './AdvancedTools';
import { AppearanceSettings, THEMES } from './AppearanceSettings';
import { CommandConsole } from './CommandConsole';
import { ConnectionSettings } from './ConnectionSettings';
import { NeedsServer, SettingRow, SettingsGroup, SettingsSection } from './layout';
import { AiSetting, CacheSetting } from './MoreSettings';
import { ServeOptions } from './ServeOptions';
import { SECTIONS, SettingsNav, sectionLabel, type SectionId } from './SettingsNav';
import { VersionInUse, WebVersions } from './WebVersions';
import type { ProfilesState } from './useProfiles';
import { t } from '../i18n';
import './settings.css';

/**
 * The sections opened since the settings were, each kept as it was (a form typed into, the console) while another is
 * open; the settings read what they show as a section first opens, and again once they are left and opened anew.
 */
function Sections({ open, pane, children }: { open: SectionId; pane: RefObject<HTMLDivElement | null>; children(id: SectionId): ReactNode }) {
  const [opened, setOpened] = useState<SectionId[]>([open]);
  useEffect(() => {
    setOpened(old => (old.includes(open) ? old : [...old, open]));
    pane.current?.scrollTo?.({ top: 0, behavior: 'instant' });
  }, [open, pane]);
  return SECTIONS.filter(section => section.id === open || opened.includes(section.id)).map(({ id }) =>
    <div key={id} className="settings-detail" hidden={id !== open}>{children(id)}</div>);
}

/** The settings show no cards of their own: the console keeps those of the page browsed before. */
export function useSettingsPage(shell: Shell, prefs: Preferences, { profiles: { profiles, failed, reload }, recheck, onSetup }: {
  profiles: ProfilesState; recheck(): void; onSetup(): void;
}): Page {
  const { backend, connected, profile, mobileDetail, setMobileDetail } = shell;
  // held here, as the page is there all the time: a theme that follows the system keeps following it
  const [theme, setTheme] = useTheme();
  const [open, setOpen] = useState<SectionId>('connection');
  const pane = useRef<HTMLDivElement>(null);
  const go = useCallback((id: SectionId) => { setOpen(id); setMobileDetail(true); }, [setMobileDetail]);
  const [consolePick, setConsolePick] = useState<string | null>(null);
  const consolePicked = useCallback(() => setConsolePick(null), []);

  const health = 'health' in backend ? backend.health : null;
  const current = profiles?.find(p => p.current);
  // a few words each, two lines at most
  const summaries: Record<SectionId, string> = {
    connection: [backendLabel(backend), health?.version].filter(Boolean).join(' · '),
    accounts: current ? [current.name, current.complete ? t('已配置') : profileState(current)].join(' · ') : failed ? t('读不到账号配置') : '',
    appearance: [t(THEMES.find(choice => choice.value === theme)!.label), prefs.realtime ? t('实时消息') : t('不接收实时消息')].join(' · '),
    serve: t('地址、端口和并发'),
    ai: profile?.aiKey ? t('已设置 API Key') : t('未设置 API Key'),
    cache: t('各部分的大小和清理'),
    web: t('正在使用 {version}', { version: WEB.version }),
    console: t('CLI 工具和任意命令'),
  };
  const offline = <NeedsServer onConnect={() => go('connection')} />;

  const section = (id: SectionId): ReactNode => {
    switch (id) {
      case 'connection':
        return <SettingsSection title={sectionLabel(id)} lead={t('连接到 weflow serve 后读取本机记录；未连接时显示示例数据。')}>
          <ConnectionSettings backend={backend} recheck={recheck} />
        </SettingsSection>;
      case 'accounts':
        return <SettingsSection title={sectionLabel(id)} lead={t('每个配置档案对应一个微信账号。切换档案后，各页重新读取这个账号的记录。')}>
          {!connected ? offline
            : profiles ? <Accounts profiles={profiles} onChanged={reload} onSetup={onSetup} />
              : failed ? <SettingsGroup><SettingRow name={t('读不到账号配置')} text={t('weflow 5.0.0-rc.7 起才能在网页里查看。')}>
                <button className="secondary-button" type="button" onClick={onSetup}>{t('配置向导')}</button></SettingRow></SettingsGroup>
                : <SettingsGroup title={t('配置档案')}><AccountsSkeleton /></SettingsGroup>}
        </SettingsSection>;
      case 'appearance':
        return <SettingsSection title={sectionLabel(id)} lead={t('只保存在这个浏览器里。')}>
          <AppearanceSettings prefs={prefs} theme={theme} setTheme={setTheme} />
        </SettingsSection>;
      case 'serve':
        return <SettingsSection title={sectionLabel(id)} lead={t('weflow serve 启动时用的选项，保存在服务端的配置文件里，重启后生效。启动命令里给出的选项优先；留空的项用灰色的默认值。')}>
          {connected ? <ServeOptions onAi={() => go('ai')} /> : offline}
        </SettingsSection>;
      case 'ai':
        return <SettingsSection title={sectionLabel(id)} lead={t('灵感信箱用的 AI 模型接口，保存在当前档案里。')}>
          {connected ? <AiSetting profile={profile} engine={health?.insight} onSaved={reload} onServe={() => go('serve')} /> : offline}
        </SettingsSection>;
      case 'cache':
        return <SettingsSection title={sectionLabel(id)} lead={t('WeFlow 为读得更快而保存的文件。清理不会动微信自己的文件，需要时会重新生成。')}>
          {connected ? <CacheSetting /> : offline}
        </SettingsSection>;
      case 'web':
        return <SettingsSection title={sectionLabel(id)} lead={t('网页和 weflow 分开发布，可以在这里换成其他版本。')}>
          {connected && health ? <WebVersions health={health} /> : <><VersionInUse />{offline}</>}
        </SettingsSection>;
      case 'console':
        return <SettingsSection title={sectionLabel(id)} lead={t('CLI 独有的工具，以及按 OpenAPI 运行任意命令的控制台。')}>
          {connected ? <>
            <SettingsGroup title={t('CLI 工具')}><AdvancedTools onOpen={setConsolePick} /></SettingsGroup>
            <SettingsGroup title={t('运行命令')}><CommandConsole pick={consolePick} onPicked={consolePicked} /></SettingsGroup>
          </> : offline}
        </SettingsSection>;
    }
  };

  const view = <BrowserPanel label={t('设置')} className="settings-browser" detail={mobileDetail} onBack={() => setMobileDetail(false)}
    back={t('返回设置')} title={sectionLabel(open)} paneLabel={sectionLabel(open)} pane={pane} detailKey={open}
    list={<SettingsNav open={open} summaries={summaries} connected={connected} onOpen={go} />}>
    <Sections open={open} pane={pane}>{section}</Sections>
  </BrowserPanel>;
  return { view };
}
