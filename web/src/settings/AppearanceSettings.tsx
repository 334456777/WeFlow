// 这个浏览器为网页保存的设置（#776）：外观有皮肤（#830）、颜色主题（#252）和减少动画（#193），浏览有实时消息
// （#201）和导出时附带的媒体（#342）。
import { MediaChips } from '../exports/ExportCards';
import { Segmented } from '../Segmented';
import type { Preferences } from '../shell/preferences';
import { SKINS, type Skin } from '../skin';
import type { ThemeChoice } from '../theme';
import { SettingRow, SettingsGroup, Toggle } from './layout';
import { msg, t } from '../i18n';

/** 可选的颜色主题和它们的标签（显示时经过 t()）。 */
export const THEMES: { value: ThemeChoice; label: string }[] = [{ value: 'system', label: msg('跟随系统') }, { value: 'light', label: msg('浅色') }, { value: 'dark', label: msg('深色') }];

/** 皮肤那一行的说明：只有一种颜色主题的皮肤，选另一种主题时仍是它那一种。 */
function skinText(skin: Skin): string {
  if (skin.themes.length !== 1) return t('与颜色主题分开选择，浅色、深色都有。');
  return t('「{skin}」只有{theme}，选另一种颜色主题时也是{theme}。', { skin: t(skin.label), theme: skin.themes[0] === 'light' ? t('浅色') : t('深色') });
}

/** 皮肤、颜色主题、减少动画、实时消息和导出时附带的媒体。 */
export function AppearanceSettings({ prefs, theme, setTheme, skin, setSkin }: {
  prefs: Preferences;
  /** 由一直挂载的设置页持有，因此跟随系统的主题在离开设置后仍随系统切换。 */
  theme: ThemeChoice; setTheme(choice: ThemeChoice): void;
  skin: Skin; setSkin(id: string): void;
}) {
  return <>
    <SettingsGroup title={t('外观')}>
      {/* 有不止一个皮肤时才可以选择 */}
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
