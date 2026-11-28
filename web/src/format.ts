// 各页面通用的数字、大小、时间和日期的写法。
import { formatDate, lang, locale, relativeDay } from './i18n';

/** 按页面的语言加上千位分隔符的数目。 */
export const n = (value: number) => value.toLocaleString(locale());

/** 以 B、KB 或 MB 为单位的文件大小。 */
export const formatSize = (bytes: number) =>
  bytes < 1024 ? bytes + ' B' : bytes < 1024 * 1024 ? (bytes / 1024).toFixed(1) + ' KB' : (bytes / 1024 / 1024).toFixed(1) + ' MB';

/** 以 B、KB、MB 或 GB 为单位的大小；没有大小、为 0 或为负时为空字符串。 */
export function formatFileSize(bytes: number | undefined): string {
  if (!bytes || bytes < 0) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return (unit === 0 ? String(value) : value.toFixed(value < 10 ? 1 : 0)) + ' ' + units[unit];
}

/** 按微信的写法显示的语音时长：`12″`、`1′05″`。 */
export function formatDuration(seconds: number | undefined): string {
  const s = Math.max(0, Math.round(seconds ?? 0));
  return s < 60 ? s + '″' : Math.floor(s / 60) + '′' + String(s % 60).padStart(2, '0') + '″';
}

const pad = (n: number) => String(n).padStart(2, '0');

/** Unix 秒数对应的本地时刻，写成 `HH:MM`。 */
export const formatClock = (seconds: number) => {
  const date = new Date(seconds * 1000);
  return pad(date.getHours()) + ':' + pad(date.getMinutes());
};

/** 消息所在的本地日期，写成 `2026-10-12`。 */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/**
 * 一天的标题：「今天」「昨天」，或者带星期的日期（不是今年时加上年份），用页面的语言：中文为 `10月12日 周一`，英文为
 * `Mon, Oct 12`。
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
