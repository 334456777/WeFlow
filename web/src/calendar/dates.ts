// 日历用的日期（#224）：按本地时间写成 `YYYY-MM-DD` 字符串。这样可以直接按文本比较先后，
// 也不会像 UTC 零点的 `Date` 那样随时区变成另一天。

import { formatDate, lang } from '../i18n';

const pad = (n: number) => String(n).padStart(2, '0');

/** 从周一开始的一周七天，按页面的语言写成窄、短、长三种名称：`一` `周一` `星期一`，`M` `Mon` `Monday`。 */
export function weekdays(): { narrow: string; short: string; long: string }[] {
  return Array.from({ length: 7 }, (_, i) => {
    const date = new Date(2024, 0, 1 + i); // 2024 年 1 月 1 日是周一
    return {
      narrow: formatDate(date, { weekday: 'narrow' }), short: formatDate(date, { weekday: 'short' }), long: formatDate(date, { weekday: 'long' }),
    };
  });
}

/** `date` 在本地时间的日期。 */
export function toDay(date: Date): string {
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/** `day` 这一天开始时的本地零点。 */
export function parseDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y, m - 1, d);
}

/** 本地时间的今天。 */
export const today = () => toDay(new Date());

/** `day` 之后 `days` 天的日期（`days` 为负数时是之前）。 */
export function addDays(day: string, days: number): string {
  const date = parseDay(day);
  date.setDate(date.getDate() + days);
  return toDay(date);
}

/** `months` 个月之后的同一天；那个月没有这一天时取它的最后一天（1 月 31 日加一个月是 2 月 28 日或 29 日）。 */
export function addMonths(day: string, months: number): string {
  const date = parseDay(day);
  const target = new Date(date.getFullYear(), date.getMonth() + months, 1);
  const last = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
  target.setDate(Math.min(date.getDate(), last));
  return toDay(target);
}

/** 星期几，周一为 0、周日为 6：一周从周一开始。 */
export const weekdayOf = (day: string) => (parseDay(day).getDay() + 6) % 7;

/** `day` 所在一周的周一。 */
export const startOfWeek = (day: string) => addDays(day, -weekdayOf(day));
/** `day` 所在一周的周日。 */
export const endOfWeek = (day: string) => addDays(day, 6 - weekdayOf(day));

/** 一个月显示的 6 周，每周从周一开始：这个月的日子和前后相邻的日子。 */
export function monthGrid(year: number, month: number): string[][] {
  const first = startOfWeek(toDay(new Date(year, month, 1)));
  return Array.from({ length: 6 }, (_, week) => Array.from({ length: 7 }, (_, d) => addDays(first, week * 7 + d)));
}

/** `day` 的年份和从 0 开始的月份。 */
export const monthOf = (day: string) => ({ year: Number(day.slice(0, 4)), month: Number(day.slice(5, 7)) - 1 });

/** `day`；它晚于 `max` 时为 `max`。 */
export const clampDay = (day: string, max?: string) => (max && day > max ? max : day);

/** 读屏软件为一天读出的文字：`2026年10月12日 星期一`、`Monday, October 12, 2026`。 */
export function dayLabel(day: string): string {
  const date = parseDay(day);
  const options = { year: 'numeric', month: 'long', day: 'numeric' } as const;
  if (lang() === 'zh') return formatDate(date, options) + ' ' + formatDate(date, { weekday: 'long' });
  return formatDate(date, { ...options, weekday: 'long' });
}

/** 把 `a`、`b` 两天按先后排好，早的在前。 */
export const ordered = (a: string, b: string): [string, string] => (a <= b ? [a, b] : [b, a]);
