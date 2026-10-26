// The page's language (#439): that of the server, which answers in the language of the machine it runs on
// (`/health`'s `lang`, docs/serve-api.md), so the page and the server's texts are in one language. The page is written
// in Chinese and its texts are their own keys: `t('正在读取消息')` is that text in Chinese, its English from `en.ts`,
// or the Chinese again where `en.ts` has none (`bun run i18n` lists those). Until a server has said, the page is in
// the language of the last one it talked to, else in Chinese.
import { useSyncExternalStore } from 'react';
import { en } from './en';

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

let current: Lang = remembered();
const TITLE = msg('WeFlow · 记录工作台');

/** What the document says of its language: `<html lang>` and the tab's title. */
function show(): void {
  document.documentElement.lang = current === 'en' ? 'en' : 'zh-CN';
  document.title = t(TITLE);
}

export const lang = (): Lang => current;

/** Shows the page in `next` (a server's `lang`; anything else is left as it is). */
export function setLang(next: unknown): void {
  if ((next !== 'zh' && next !== 'en') || next === current) return;
  current = next;
  show();
  try {
    localStorage.setItem(KEPT, next);
  } catch {
    // not kept: the next visit starts in Chinese until the server says
  }
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
 * { n })` is `'{n} groups'` in `en.ts`.
 */
export function t(text: string, vars?: Record<string, string | number>): string {
  const said = current === 'en' ? (en[text] ?? text) : text;
  return vars ? said.replace(/\{(\w+)\}/g, (all, name: string) => (name in vars ? String(vars[name]) : all)) : said;
}

show();
