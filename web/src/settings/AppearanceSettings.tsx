// What this browser keeps for the page (#776): how it looks (the skin, #830; the colour theme, #252; reduced motion,
// #193) and how it browses (live messages, #201; the media an export copies, #342).
import { MediaChips } from '../exports/ExportCards';
import { Segmented } from '../Segmented';
import type { Preferences } from '../shell/preferences';
import { SKINS, type Skin } from '../skin';
import type { ThemeChoice } from '../theme';
import { SettingRow, SettingsGroup, Toggle } from './layout';
import { msg, t } from '../i18n';

// shown through t()
export const THEMES: { value: ThemeChoice; label: string }[] = [{ value: 'system', label: msg('跟随系统') }, { value: 'light', label: msg('浅色') }, { value: 'dark', label: msg('深色') }];

/** What the row of the skins says: that a skin of one theme keeps it in the other. */
function skinText(skin: Skin): string {
  if (skin.themes.length !== 1) return t('与颜色主题分开选择，浅色、深色都有。');
  return t('「{skin}」只有{theme}，选另一种颜色主题时也是{theme}。', { skin: t(skin.label), theme: skin.themes[0] === 'light' ? t('浅色') : t('深色') });
}

/** The skin, the theme, reduced motion, live messages and the media an export copies. */
export function AppearanceSettings({ prefs, theme, setTheme, skin, setSkin }: {
  prefs: Preferences;
  /** Held by the settings page, which is there all the time: a theme that follows the system keeps following it. */
  theme: ThemeChoice; setTheme(choice: ThemeChoice): void;
  skin: Skin; setSkin(id: string): void;
}) {
  return <>
    <SettingsGroup title={t('外观')}>
      {/* a choice once there is more than one skin */}
      {SKINS.length > 1 && <SettingRow name={t('皮肤')} text={skinText(skin)} wide>
        <Segmented className="skin-group" count={SKINS.length} index={SKINS.indexOf(skin)} role="group" aria-label={t('皮肤')}>
          {SKINS.map(choice => <button type="button" key={choice.id} aria-pressed={skin === choice} onClick={() => setSkin(choice.id)}>{t(choice.label)}</button>)}
        </Segmented>
      </SettingRow>}
      <SettingRow name={t('颜色主题')} text={t('跟随系统时随系统的浅色、深色模式切换。')} wide>
        <Segmented className="theme-group" count={THEMES.length} index={THEMES.findIndex(choice => choice.value === theme)} role="group" aria-label={t('颜色主题')}>
          {THEMES.map(choice => <button type="button" key={choice.value} aria-pressed={theme === choice.value} onClick={() => setTheme(choice.value)}>{t(choice.label)}</button>)}
        </Segmented>
      </SettingRow>
      <SettingRow name={t('减少动画')} text={t('所有过渡立即完成，状态提示保留。也跟随系统的「减少动态效果」。')}>
        <Toggle label={t('减少动画')} checked={prefs.reducedMotion} onChange={prefs.setReducedMotion} />
      </SettingRow>
    </SettingsGroup>
    <SettingsGroup title={t('浏览')}>
      <SettingRow name={t('实时消息')} text={t('微信收到消息后几秒内显示在会话列表和打开的会话里。关掉后不再接收，左下角显示「已连接」而不是「实时」。')}>
        <Toggle label={t('实时消息')} checked={prefs.realtime} onChange={prefs.setRealtime} />
      </SettingRow>
      <SettingRow name={t('默认附带的媒体')} text={t('会话页导出时把选中的媒体放进 media 文件夹，和导出文件一起打包成 zip。默认都不选。')} wide>
        <MediaChips kinds={prefs.media} label={t('默认附带的媒体')} onChange={prefs.setMedia} />
      </SettingRow>
    </SettingsGroup>
  </>;
}
