// 网页的语言（#439）：跟随服务端，服务端按它所在机器的语言回答（`/health` 的 `lang`，见 Wiki 的「命令 API」页），
// 网页和服务端的文字因此是同一种语言。网页用中文写成，文字本身就是键：`t('正在读取消息')` 在中文下就是这句中文，
// 英文取自 `en.ts`，`en.ts` 里没有时仍显示中文（`bun run i18n` 会列出这些文字）。服务端给出语言之前，
// 网页用上一次连接的服务端的语言，没有时用中文。英文是单独的 chunk，第一次需要时才下载：中文的网页从不下载它。
import { useSyncExternalStore } from 'react';

/** 网页支持的语言。 */
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

// 要求的语言（保存下来供下次访问使用），以及网页显示所用的语言：英文的文字下载完之后才是英文
let wanted: Lang = remembered();
let current: Lang = 'zh';
let en: Record<string, string> = {};
let english: Promise<boolean> | null = null;

/** 下载一次英文；下载失败时为 false（网页保持中文，之后的调用会再试）。 */
function loadEnglish(): Promise<boolean> {
  english ??= import('./en').then(m => { en = m.en; return true; }, () => { english = null; return false; });
  return english;
}

const TITLE = msg('WeFlow · 记录工作台');

/** 更新文档里随语言变化的地方：`<html lang>` 和标签页的标题。 */
function show(): void {
  document.documentElement.lang = current === 'en' ? 'en' : 'zh-CN';
  document.title = t(TITLE);
}

/** 网页当前的语言。 */
export const lang = (): Lang => current;

/** 网页当前的语言对应的 locale，供 `Intl` 和 `toLocaleString` 使用。 */
export const locale = (): string => (current === 'en' ? 'en-US' : 'zh-CN');

const formats = new Map<string, Intl.DateTimeFormat>();

/** 用网页的语言写出的 `date`：由 `Intl.DateTimeFormat` 按 `options` 格式化（`10月12日`、`Oct 12`）。 */
export function formatDate(date: Date, options: Intl.DateTimeFormatOptions): string {
  const key = locale() + JSON.stringify(options);
  let format = formats.get(key);
  if (!format) formats.set(key, (format = new Intl.DateTimeFormat(locale(), options)));
  return format.format(date);
}

/** 用网页的语言写出从今天算起的某一天：0 为「今天」（Today），-1 为「昨天」（Yesterday）。 */
export function relativeDay(offset: number): string {
  const word = new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' }).format(offset, 'day');
  return word.charAt(0).toUpperCase() + word.slice(1);
}

let drawn: Promise<void> = Promise.resolve();

/**
 * 用 `next`（服务端的 `lang`；其他值不改变语言）显示网页，英文要等文字下载完才切换；网页用这种语言绘制后完成。
 */
export function setLang(next: unknown): Promise<void> {
  drawn = draw(next);
  return drawn;
}

/** 网页用最后要求的语言（一开始是上次访问时的语言）绘制后完成。 */
export const languageReady = (): Promise<void> => drawn;

async function draw(next: unknown): Promise<void> {
  if (next !== 'zh' && next !== 'en') return;
  if (next !== wanted) {
    wanted = next;
    try {
      localStorage.setItem(KEPT, next);
    } catch {
      // 无法保存：下次访问时先用中文，直到服务端给出语言
    }
  }
  if (next === 'en' && !(await loadEnglish())) return;
  // 英文还在下载时又要求了另一种语言
  if (wanted !== next || current === next) return;
  current = next;
  show();
  for (const listener of listeners) listener();
}

/** 网页的语言，语言变化时重新渲染。 */
export const useLang = (): Lang =>
  useSyncExternalStore(listener => (listeners.add(listener), () => listeners.delete(listener)), lang, lang);

/** 标出原样保存（如表格里的标签）、到显示时才经过 `t()` 的文字。 */
export function msg(text: string): string {
  return text;
}

/**
 * 用网页的语言写出的 `text`，其中的 `{name}` 换成 `vars` 里的值。英文用同样的变量名：`t('共 {n} 个群聊', { n })`
 * 在 `en.ts` 里是 `'{n} {n|group|groups}'`，`{n|one|other}` 分别是数目为 1 和其他数目时的词。
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
