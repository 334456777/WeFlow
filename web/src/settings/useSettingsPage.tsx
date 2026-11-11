// The settings (#214): the connection, the accounts, the token, how the server runs and what the page itself
// prefers; apart from them, the CLI's own tools and the command console (#217).
import { useCallback, useEffect, useState } from 'react';
import { Gear, Plugs } from '@phosphor-icons/react';
import { WEB, type Health } from '../api/backend';
import { onAuth, setToken, token } from '../api/client';
import { MediaChips } from '../exports/ExportCards';
import { Segmented } from '../Segmented';
import { backendDetail } from '../shell/Connection';
import type { Page, Shell } from '../shell/page';
import { ScrollPage } from '../shell/panels';
import type { Preferences } from '../shell/preferences';
import { useTheme, type ThemeChoice } from '../theme';
import { TokenForm } from '../TokenForm';
import { Accounts, AccountsSkeleton } from './Accounts';
import { AdvancedTools } from './AdvancedTools';
import { CommandConsole } from './CommandConsole';
import { AiSetting, CacheSetting } from './MoreSettings';
import { ServeOptions } from './ServeOptions';
import { WebVersions } from './WebVersions';
import type { ProfilesState } from './useProfiles';
import { msg, t } from '../i18n';

// shown through t()
const THEMES: { value: ThemeChoice; label: string }[] = [{ value: 'system', label: msg('跟随系统') }, { value: 'light', label: msg('浅色') }, { value: 'dark', label: msg('深色') }];

/** How the server runs (#214): its limits and the insight engine, when it says them. */
function serverDetail(health: Health): string | null {
  const { limits, insight } = health;
  if (!limits && insight === undefined) return null;
  const parts: string[] = [];
  if (limits?.maxTasks !== undefined) parts.push(t('同时运行任务 {n} 个', { n: limits.maxTasks }));
  if (limits?.maxReads !== undefined) parts.push(t('读取 {n} 个', { n: limits.maxReads }));
  if (limits?.maxMedia !== undefined) parts.push(t('媒体解码 {n} 个', { n: limits.maxMedia }));
  if (insight !== undefined) parts.push(insight ? t('灵感引擎已开启') : t('灵感引擎未开启'));
  return parts.join(' · ');
}

/** The settings show no cards of their own: the console keeps those of the page browsed before. */
export function useSettingsPage(shell: Shell, prefs: Preferences, { profiles: { profiles, failed, reload }, recheck, onSetup }: {
  profiles: ProfilesState; recheck(): void; onSetup(): void;
}): Page {
  const { backend, connected, profile } = shell;
  const [theme, setTheme] = useTheme();
  // whether this tab holds a token (#214); never its value
  const [tokenSaved, setTokenSaved] = useState(() => Boolean(token()));
  useEffect(() => onAuth(event => { if (event === 'token') setTokenSaved(Boolean(token())); }), []);
  // the usual settings, or the CLI's own tools and the command console (#217)
  const [tab, setTab] = useState<'general' | 'advanced'>('general');
  const [consolePick, setConsolePick] = useState<string | null>(null);
  const consolePicked = useCallback(() => setConsolePick(null), []);
  const view = <ScrollPage label={t('设置')} icon={Gear} tabs={
    <Segmented className="settings-tabs" count={2} index={tab === 'general' ? 0 : 1} role="group" aria-label={t('设置分组')}>
      <button type="button" aria-pressed={tab === 'general'} onClick={() => setTab('general')}>{t('常规')}</button>
      <button type="button" aria-pressed={tab === 'advanced'} onClick={() => setTab('advanced')}>{t('高级')}</button>
    </Segmented>}>
    {tab === 'advanced' ? <>
      <p className="muted">{t('CLI 独有的工具，以及按 OpenAPI 运行任意命令的控制台。')}</p>
      {connected
        ? <><AdvancedTools onOpen={setConsolePick} /><h2 className="settings-subheading">{t('命令控制台')}</h2><CommandConsole pick={consolePick} onPicked={consolePicked} /></>
        : <p className="settings-footnote">{t('连接 weflow serve 后才能使用。')}</p>}
    </> : <>
      <p className="muted">{t('管理连接和浏览偏好。')}</p>
      <div className="setting-row"><div><h2>weflow serve</h2><p>{backendDetail(backend)}</p></div><button className="secondary-button" type="button" onClick={recheck}>{t('重新检测')}</button></div>
      {connected && (profiles
        ? <div className="setting-row setting-accounts"><div><h2>{t('账号')}</h2><p>{t('每个配置档案对应一个微信账号。切换档案后，各页重新读取这个账号的记录。')}</p>
          <Accounts profiles={profiles} onChanged={reload} onSetup={onSetup} /></div></div>
        : failed
          ? <div className="setting-row"><div><h2>{t('账号')}</h2><p>{t('读不到账号配置：weflow 5.0.0-rc.7 起才能在网页里查看。')}</p></div>
            <button className="secondary-button" type="button" onClick={onSetup}>{t('配置向导')}</button></div>
          : <div className="setting-row setting-accounts"><div><h2>{t('账号')}</h2><p>{t('每个配置档案对应一个微信账号。切换档案后，各页重新读取这个账号的记录。')}</p><AccountsSkeleton /></div></div>)}
      {'health' in backend && <div className="setting-row"><div><h2>{t('weflow 版本')}</h2>
        <p>weflow {backend.health.version} · API {backend.health.apiVersion}</p>
        {backend.state === 'incompatible' && <p className="setting-warning">{backend.reason}。{backend.advice}</p>}
      </div><span className={'source-name' + (backend.state === 'incompatible' ? ' warning' : '')}>{backend.state === 'incompatible' ? t('不适用于这个网页') : t('适用于这个网页')}</span></div>}
      {connected && 'health' in backend ? <WebVersions health={backend.health} /> : <div className="setting-row"><div><h2>{t('网页版本')}</h2><p>{t('需要 API {apiVersion}，weflow {minBackendVersion} 或更新的版本。', { apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion })}</p></div><span className="source-name"><Plugs size={14} />{WEB.version}</span></div>}
      <div className="setting-row setting-token"><div><h2>{t('令牌')}</h2>
        <p>{tokenSaved
          ? t('这个标签页保存了令牌，关闭后清除。需要更换时在这里粘贴，只保存在这个标签页里。')
          : t('没有保存令牌：从 weflow serve 启动时给出的地址打开网页，可自动连接。需要更换时在这里粘贴，只保存在这个标签页里。')}</p>
        <TokenForm label={t('新的令牌')} />
      </div>{tokenSaved && <button className="secondary-button" type="button" onClick={() => setToken(null)}>{t('清除')}</button>}</div>
      {'health' in backend && serverDetail(backend.health) && <div className="setting-row"><div><h2>{t('运行方式')}</h2><p>{serverDetail(backend.health)}</p></div></div>}
      {connected && <ServeOptions />}
      {connected && <><AiSetting profile={profile} onSaved={reload} /><CacheSetting /></>}
      <div className="setting-row"><div><h2>{t('实时消息')}</h2><p>{t('微信收到消息后几秒内显示在会话列表和打开的会话里。关掉后不再接收，左下角显示「已连接」而不是「实时」。')}</p></div><label className="toggle"><input type="checkbox" aria-label={t('实时消息')} checked={prefs.realtime} onChange={event => prefs.setRealtime(event.target.checked)} /><span /></label></div>
      <div className="setting-row setting-choice"><div><h2>{t('默认附带的媒体')}</h2><p>{t('会话页导出时把选中的媒体放进 media 文件夹，和导出文件一起打包成 zip。默认都不选。')}</p></div>
        <MediaChips kinds={prefs.media} label={t('默认附带的媒体')} onChange={prefs.setMedia} /></div>
      <div className="setting-row setting-choice"><div><h2>{t('颜色主题')}</h2><p>{t('跟随系统时随系统的浅色、深色模式切换。只保存在这个浏览器里。')}</p></div>
        <Segmented className="theme-group" count={THEMES.length} index={THEMES.findIndex(choice => choice.value === theme)} role="group" aria-label={t('颜色主题')}>
          {THEMES.map(choice => <button type="button" key={choice.value} aria-pressed={theme === choice.value} onClick={() => setTheme(choice.value)}>{t(choice.label)}</button>)}
        </Segmented></div>
      <div className="setting-row"><div><h2>{t('减少动画')}</h2><p>{t('所有过渡立即完成，状态提示保留。也跟随系统的「减少动态效果」。')}</p></div><label className="toggle"><input type="checkbox" aria-label={t('减少动画')} checked={prefs.reducedMotion} onChange={event => prefs.setReducedMotion(event.target.checked)} /><span /></label></div>
      <small className="settings-footnote">{t('连接到 weflow serve 后读取本机记录；未连接时显示示例数据。')}</small>
    </>}
  </ScrollPage>;
  return { view };
}
