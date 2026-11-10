// Days as `YYYY-MM-DD` strings in local time, for the calendar (#224): they compare as text, and a day never
// shifts with the time zone the way a Date at midnight UTC would.

import { formatDate, lang } from '../i18n';

const pad = (n: number) => String(n).padStart(2, '0');

/** The days of a week from Monday, in the page's language: 一 周一 星期一, M Mon Monday. */
export function weekdays(): { narrow: string; short: string; long: string }[] {
  return Array.from({ length: 7 }, (_, i) => {
    const date = new Date(2024, 0, 1 + i); // a Monday
    return {
      narrow: formatDate(date, { weekday: 'narrow' }), short: formatDate(date, { weekday: 'short' }), long: formatDate(date, { weekday: 'long' }),
    };
  });
}

export function toDay(date: Date): string {
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

export function parseDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y, m - 1, d);
}

export const today = () => toDay(new Date());

export function addDays(day: string, days: number): string {
  const date = parseDay(day);
  date.setDate(date.getDate() + days);
  return toDay(date);
}

/** The same day `months` later, or the month's last day when it is shorter (31 Jan + 1 month = 28 or 29 Feb). */
export function addMonths(day: string, months: number): string {
  const date = parseDay(day);
  const target = new Date(date.getFullYear(), date.getMonth() + months, 1);
  const last = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
  target.setDate(Math.min(date.getDate(), last));
  return toDay(target);
}

/** 0 for Monday to 6 for Sunday: the week starts on Monday. */
export const weekdayOf = (day: string) => (parseDay(day).getDay() + 6) % 7;

export const startOfWeek = (day: string) => addDays(day, -weekdayOf(day));
export const endOfWeek = (day: string) => addDays(day, 6 - weekdayOf(day));

/** The 6 weeks shown for a month, Monday first: the month's days and the ones around it. */
export function monthGrid(year: number, month: number): string[][] {
  const first = startOfWeek(toDay(new Date(year, month, 1)));
  return Array.from({ length: 6 }, (_, week) => Array.from({ length: 7 }, (_, d) => addDays(first, week * 7 + d)));
}

export const monthOf = (day: string) => ({ year: Number(day.slice(0, 4)), month: Number(day.slice(5, 7)) - 1 });

export const clampDay = (day: string, max?: string) => (max && day > max ? max : day);

/** What a screen reader says for a day: `2026年10月12日 星期一`, `Monday, October 12, 2026`. */
export function dayLabel(day: string): string {
  const date = parseDay(day);
  const options = { year: 'numeric', month: 'long', day: 'numeric' } as const;
  if (lang() === 'zh') return formatDate(date, options) + ' ' + formatDate(date, { weekday: 'long' });
  return formatDate(date, { ...options, weekday: 'long' });
}

/** The days from `a` to `b` in order, whichever comes first. */
export const ordered = (a: string, b: string): [string, string] => (a <= b ? [a, b] : [b, a]);
