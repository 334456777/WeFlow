// 皮肤（#784、#830）：页面的外观，与浅色、深色主题（theme.ts）分开选择，可以和任一种主题搭配。
// 默认皮肤新拟态随主样式表（styles/neumorphism.css）一起加载；其他皮肤各有自己的样式表，第一次选中时才下载，
// 与英文的翻译（i18n）一样。选择保存在这个浏览器的 localStorage 里，与主题的选择分开；生效的皮肤是 <html> 上的
// `data-skin`。index.html 在第一次绘制之前按同样的规则设好它；选的不是默认皮肤时先隐藏页面（`data-skin-pending`），
// 等它的样式表到了再显示，页面因此不会先以默认皮肤出现。
//
// 皮肤的样式表画的图片（装饰、纹理）随样式表一起提供：构建时把小于 4 KB 的图片以 data: 的形式写进样式表，
// 更大的放在旁边的 /ui/assets/ 下，皮肤的规则显示它时才下载；`url(…?inline)` 不论大小都把图片写进样式表，
// 导出的报告里显示的图片必须这样写（reports/reportFiles.ts 只复制样式表，不复制旁边的文件）。
import { useState, useSyncExternalStore } from 'react';
import { msg } from './i18n';

/** 皮肤可以有的配色主题。 */
export type ThemeName = 'light' | 'dark';

/** 设置里可选的一个皮肤。 */
export interface Skin {
  /** `data-skin` 的值，也是保存下来的名字。 */
  id: string;
  /** 设置里显示的名称，显示时经过 t()。 */
  label: string;
  /** 皮肤有的主题。选了它没有的主题时，页面保持它有的那种主题的颜色（它的样式表只在 `:root[data-skin]` 下设置颜色，
   * 默认皮肤的深色颜色无法覆盖它），设置里也会说明。 */
  themes: readonly ThemeName[];
  /** 以文字形式下载它的样式表，写成 `() => import('./styles/skins/<id>.css?inline')`：一个纯脚本的 chunk，
   * 构建时加载它不会预加载别的东西（plugins/plainImport.ts）。默认皮肤随主样式表加载，没有这一项。 */
  sheet?: () => Promise<{ default: string }>;
  /** 用外形层画按钮、控件、可选项和列表行（styles/roles.css，#831）：这个皮肤生效时 <html> 上有 `data-shapes`。 */
  shapes?: boolean;
}

/** 默认皮肤的 ID：新拟态，样式表是 styles/neumorphism.css。 */
export const DEFAULT_SKIN = 'neumorphism';

/** 可选的皮肤，默认皮肤在最前。皮肤的样式表是 styles/skins/<id>.css，其中每条规则都在 `skin` 层里、写在
 * `:root[data-skin='<id>']` 之下（styles/layers.test.ts 检查）。 */
export const SKINS: Skin[] = [
  { id: DEFAULT_SKIN, label: msg('新拟态'), themes: ['light', 'dark'] },
  { id: 'sticker', label: msg('贴纸'), themes: ['light', 'dark'], sheet: () => import('./styles/skins/sticker.css?inline'), shapes: true },
];

/** 选择的皮肤保存在 `localStorage` 里所用的键。 */
export const SKIN_KEY = 'weflow.skin';

const skinOf = (id: string | null | undefined): Skin => SKINS.find(skin => skin.id === id) ?? SKINS[0];

/** 这个浏览器里保存的皮肤；没有保存，或保存的皮肤这个页面没有时为默认皮肤。 */
export function savedSkin(): Skin {
  try {
    return skinOf(localStorage.getItem(SKIN_KEY));
  } catch {
    return SKINS[0];
  }
}

const sheets = new WeakMap<Skin, Promise<boolean>>();

/** 下载一次皮肤的样式表并加到页面上，它只在这个皮肤生效时起作用（规则都在 `:root[data-skin]` 之下）；
 * 下载失败时为 false，之后的调用会再试。 */
function fetchSheet(skin: Skin): Promise<boolean> {
  if (!skin.sheet) return Promise.resolve(true);
  let fetched = sheets.get(skin);
  if (!fetched) {
    fetched = skin.sheet().then(({ default: css }) => {
      const style = document.createElement('style');
      style.dataset.skin = skin.id;
      style.textContent = css;
      document.head.append(style);
      return true;
    }, () => { sheets.delete(skin); return false; });
    sheets.set(skin, fetched);
  }
  return fetched;
}

let asked = 0;
const putOn = new Set<() => void>();

/** 页面上生效的皮肤（`data-skin`），换上另一个皮肤时更新：供按皮肤用脚本绘制的部分使用（Backdrop.tsx）。 */
export function useSkinOn(): string {
  const read = () => document.documentElement.dataset.skin ?? DEFAULT_SKIN;
  return useSyncExternalStore(listener => { putOn.add(listener); return () => { putOn.delete(listener); }; }, read, read);
}

/** 样式表到了以后把 `skin` 换到页面上，样式表下载失败时换上默认皮肤；第一次绘制时隐藏了页面的话，同时显示页面。
 * 之后的调用要求的皮肤优先于这一次。结果是实际换上的皮肤。 */
export async function applySkin(skin: Skin): Promise<Skin> {
  const turn = ++asked;
  const fetched = await fetchSheet(skin);
  const root = document.documentElement;
  if (turn !== asked) return skinOf(root.dataset.skin);
  const shown = fetched ? skin : SKINS[0];
  root.dataset.skin = shown.id;
  if (shown.shapes) root.dataset.shapes = ''; else delete root.dataset.shapes;
  delete root.dataset.skinPending;
  for (const listener of putOn) listener();
  return shown;
}

/** 把保存的皮肤换到页面上；main.tsx 在第一次绘制页面之前等它完成。 */
export const skinReady = (): Promise<Skin> => applySkin(savedSkin());

/** 选择的皮肤和改选的函数：皮肤换到页面上并保存下来。 */
export function useSkin(): [Skin, (id: string) => void] {
  const [chosen, setChosen] = useState<Skin>(() => skinOf(document.documentElement.dataset.skin ?? savedSkin().id));
  const choose = (id: string) => {
    const next = skinOf(id);
    setChosen(next);
    try {
      if (next.id === DEFAULT_SKIN) localStorage.removeItem(SKIN_KEY);
      else localStorage.setItem(SKIN_KEY, next.id);
    } catch {
      // 无法保存：这个选择只在当前页面里有效
    }
    // 样式表下载失败时页面保持默认皮肤，设置里随之显示默认皮肤
    void applySkin(next).then(shown => { if (shown !== next) setChosen(current => (current === next ? shown : current)); });
  };
  return [chosen, choose];
}
