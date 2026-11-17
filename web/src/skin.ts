// The skin (#784, #830): what the page looks like, chosen apart from the light or dark theme (theme.ts) and with
// either. The default skin, neumorphism, comes with the main style sheet (styles/neumorphism.css); each other one is a
// style sheet of its own, fetched the first time it is chosen, as the English is (i18n). The choice is kept in this
// browser's localStorage apart from the theme's; the skin in force is `data-skin` on <html>. index.html sets it before
// the first paint by the same rules and, for a skin other than the default, hides the page (`data-skin-pending`) until
// its sheet is here, so the page never shows in the default skin first.
//
// The pictures a skin's sheet draws (a decoration, a texture) come with it: the build puts one under 4 KB into the
// sheet as data:, a larger one beside it under /ui/assets/, fetched once a rule of the skin's shows it; `url(…?inline)`
// puts a picture into the sheet whatever its size, as one shown in an exported report must be (reports/reportFiles.ts
// copies the sheets, not the files beside them).
import { useState, useSyncExternalStore } from 'react';
import { msg } from './i18n';

export type ThemeName = 'light' | 'dark';

export interface Skin {
  /** `data-skin`, and the name kept. */
  id: string;
  /** Its name in the settings, shown through t(). */
  label: string;
  /** The themes it has. In another the page keeps the colours of the one it has (its sheet sets them under
   * `:root[data-skin]` alone, which the default skin's dark colours cannot override), and the settings say so. */
  themes: readonly ThemeName[];
  /** Fetches its style sheet as text, `() => import('./styles/skins/<id>.css?inline')`: a chunk of plain script, which
   * the build loads without preloading anything (plugins/plainImport.ts). None for the default skin, which comes with
   * the main sheet. */
  sheet?: () => Promise<{ default: string }>;
  /** Draws the buttons, controls, options and rows with shape layers (styles/roles.css, #831): `data-shapes` on
   * <html> while it is on. */
  shapes?: boolean;
}

export const DEFAULT_SKIN = 'neumorphism';

/** The skins to choose from, the default first. A skin's sheet is styles/skins/<id>.css, every rule of it in the
 * `skin` layer under `:root[data-skin='<id>']` (styles/layers.test.ts). */
export const SKINS: Skin[] = [
  { id: DEFAULT_SKIN, label: msg('新拟态'), themes: ['light', 'dark'] },
];

export const SKIN_KEY = 'weflow.skin';

const skinOf = (id: string | null | undefined): Skin => SKINS.find(skin => skin.id === id) ?? SKINS[0];

/** The skin kept in this browser; the default for none, or for one this page does not have. */
export function savedSkin(): Skin {
  try {
    return skinOf(localStorage.getItem(SKIN_KEY));
  } catch {
    return SKINS[0];
  }
}

const sheets = new WeakMap<Skin, Promise<boolean>>();

/** Fetches a skin's sheet once and adds it to the page, where it holds while the skin is on (its rules are under
 * `:root[data-skin]`); false when it could not be fetched (and a later call tries again). */
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

/** The skin on the page (`data-skin`), anew when another is put on: for what is drawn by script as the skin says
 * (Backdrop.tsx). */
export function useSkinOn(): string {
  const read = () => document.documentElement.dataset.skin ?? DEFAULT_SKIN;
  return useSyncExternalStore(listener => { putOn.add(listener); return () => { putOn.delete(listener); }; }, read, read);
}

/** Puts `skin` on the page once its sheet is here, or the default skin if the sheet cannot be fetched; and shows the
 * page if the first paint hid it. What a later call asks for wins over this one. Resolves to the skin put on. */
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

/** The kept skin put on the page; main.tsx waits for it before the page is first drawn. */
export const skinReady = (): Promise<Skin> => applySkin(savedSkin());

/** The chosen skin, put on the page and kept. */
export function useSkin(): [Skin, (id: string) => void] {
  const [chosen, setChosen] = useState<Skin>(() => skinOf(document.documentElement.dataset.skin ?? savedSkin().id));
  const choose = (id: string) => {
    const next = skinOf(id);
    setChosen(next);
    try {
      if (next.id === DEFAULT_SKIN) localStorage.removeItem(SKIN_KEY);
      else localStorage.setItem(SKIN_KEY, next.id);
    } catch {
      // not kept: the choice holds for this page
    }
    // a sheet that cannot be fetched leaves the default skin on, which the settings then show
    void applySkin(next).then(shown => { if (shown !== next) setChosen(current => (current === next ? shown : current)); });
  };
  return [chosen, choose];
}
