// How numbers, sizes, times and days read on every page.
import { formatDate, lang, locale, relativeDay } from './i18n';

/** A count with the page's separators. */
export const n = (value: number) => value.toLocaleString(locale());

/** A file's size in B, KB or MB. */
export const formatSize = (bytes: number) =>
  bytes < 1024 ? bytes + ' B' : bytes < 1024 * 1024 ? (bytes / 1024).toFixed(1) + ' KB' : (bytes / 1024 / 1024).toFixed(1) + ' MB';

/** A size in B, KB, MB or GB; empty for none. */
export function formatFileSize(bytes: number | undefined): string {
  if (!bytes || bytes < 0) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return (unit === 0 ? String(value) : value.toFixed(value < 10 ? 1 : 0)) + ' ' + units[unit];
}

const pad = (n: number) => String(n).padStart(2, '0');

/** The local time of Unix seconds, `HH:MM`. */
export const formatClock = (seconds: number) => {
  const date = new Date(seconds * 1000);
  return pad(date.getHours()) + ':' + pad(date.getMinutes());
};

/** The local day of a message, as `2026-10-12`. */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/**
 * The heading of a day: 今天, 昨天, or the date with its weekday (and the year when it is not this year), in the page's
 * language: `10月12日 周一` in Chinese, `Mon, Oct 12` in English.
 */
export function formatDayHeading(day: string, now = new Date()): string {
  const [y, m, d] = day.split('-').map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  if (days === 0 || days === 1) return relativeDay(-days);
  const year = y === now.getFullYear() ? undefined : 'numeric';
  if (lang() === 'zh') return formatDate(date, { year, month: 'long', day: 'numeric' }) + ' ' + formatDate(date, { weekday: 'short' });
  return formatDate(date, { year, month: 'short', day: 'numeric', weekday: 'short' });
}
