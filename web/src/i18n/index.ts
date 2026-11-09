// The page's language (#439): that of the server, which answers in the language of the machine it runs on
// (`/health`'s `lang`, the wiki's command API page), so the page and the server's texts are in one language. The page is written
// in Chinese and its texts are their own keys: `t('正在读取消息')` is that text in Chinese, its English from `en.ts`,
// or the Chinese again where `en.ts` has none (`bun run i18n` lists those). Until a server has said, the page is in
// the language of the last one it talked to, else in Chinese. The English is a chunk of its own, fetched the first time
// it is wanted: a page in Chinese never downloads it.
import { useSyncExternalStore } from 'react';

export type Lang = 'zh' | 'en';

const KEPT = 'weflow.lang';
const listeners = new Set<() => void>();

function remembered(): Lang {
  try {
    return localStorage.getItem(KEPT) === 'en' ? 'en' : 'zh';
  } catch {
    return 'zh';
  }
}

// the language asked for (kept for the next visit), and the one the page is drawn in: English once its words are here
let wanted: Lang = remembered();
let current: Lang = 'zh';
let en: Record<string, string> = {};
let english: Promise<boolean> | null = null;

/** Fetches the English once; false when it could not be fetched (the page stays in Chinese, and a later call tries again). */
function loadEnglish(): Promise<boolean> {
  english ??= import('./en').then(m => { en = m.en; return true; }, () => { english = null; return false; });
  return english;
}

const TITLE = msg('WeFlow · 记录工作台');

/** What the document says of its language: `<html lang>` and the tab's title. */
function show(): void {
  document.documentElement.lang = current === 'en' ? 'en' : 'zh-CN';
  document.title = t(TITLE);
}

export const lang = (): Lang => current;

/** The page's language as a locale, for `Intl` and `toLocaleString`. */
export const locale = (): string => (current === 'en' ? 'en-US' : 'zh-CN');

const formats = new Map<string, Intl.DateTimeFormat>();

/** `date` in the page's language, as `Intl.DateTimeFormat` writes it with `options` (`10月12日`, `Oct 12`). */
export function formatDate(date: Date, options: Intl.DateTimeFormatOptions): string {
  const key = locale() + JSON.stringify(options);
  let format = formats.get(key);
  if (!format) formats.set(key, (format = new Intl.DateTimeFormat(locale(), options)));
  return format.format(date);
}

/** A day counted from today in the page's language: 今天, 昨天 (Today, Yesterday) for 0 and -1. */
export function relativeDay(offset: number): string {
  const word = new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' }).format(offset, 'day');
  return word.charAt(0).toUpperCase() + word.slice(1);
}

let drawn: Promise<void> = Promise.resolve();

/**
 * Shows the page in `next` (a server's `lang`; anything else is left as it is), English once its words are fetched;
 * settles when the page is drawn in it.
 */
export function setLang(next: unknown): Promise<void> {
  drawn = draw(next);
  return drawn;
}

/** Settles once the page is drawn in the language asked for last (at first, that of the last visit). */
export const languageReady = (): Promise<void> => drawn;

async function draw(next: unknown): Promise<void> {
  if (next !== 'zh' && next !== 'en') return;
  if (next !== wanted) {
    wanted = next;
    try {
      localStorage.setItem(KEPT, next);
    } catch {
      // not kept: the next visit starts in Chinese until the server says
    }
  }
  if (next === 'en' && !(await loadEnglish())) return;
  // asked again for the other language while the English was on its way
  if (wanted !== next || current === next) return;
  current = next;
  show();
  for (const listener of listeners) listener();
}

/** The page's language, rendering again when it changes. */
export const useLang = (): Lang =>
  useSyncExternalStore(listener => (listeners.add(listener), () => listeners.delete(listener)), lang, lang);

/** Marks a text kept as it is (a label in a table, say) and passed through `t()` where it is shown. */
export function msg(text: string): string {
  return text;
}

/**
 * `text` in the page's language, its `{name}`s replaced by `vars`. The English keeps the same names: `t('共 {n} 个群聊',
 * { n })` is `'{n} {n|group|groups}'` in `en.ts`, where `{n|one|other}` is the word for a count of 1 and for any other.
 */
export function t(text: string, vars?: Record<string, string | number>): string {
  const said = current === 'en' ? (en[text] ?? text) : text;
  if (!vars) return said;
  return said
    .replace(/\{(\w+)\|([^|{}]*)\|([^|{}]*)\}/g, (all, name: string, one: string, other: string) =>
      (name in vars ? (Number(vars[name]) === 1 ? one : other) : all))
    .replace(/\{(\w+)\}/g, (all, name: string) => (name in vars ? String(vars[name]) : all));
}

show();

void setLang(wanted);
