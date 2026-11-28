// 示例数据的时间，从打开页面的时刻往回算：示例记录因此总是最近的。

/** 加载示例数据时的 Unix 秒数。 */
export const NOW = Math.floor(Date.now() / 1000);

/** 打开页面之前 `days` 天 `hours` 小时 `minutes` 分钟的 Unix 秒数。 */
export const ago = (days: number, hours = 0, minutes = 0): number => NOW - ((days * 24 + hours) * 60 + minutes) * 60;

/**
 * 今天之前 `days` 天的 `HH:MM` 时刻的 Unix 秒数：用于在一天中特定时刻说的话（早上好、深夜），
 * 放在今天之前的某一天，这样总是在过去。
 */
export function at(days: number, clock: string): number {
  const [hours, minutes] = clock.split(':').map(Number);
  const date = new Date(NOW * 1000);
  date.setDate(date.getDate() - days);
  date.setHours(hours, minutes, 0, 0);
  return Math.floor(date.getTime() / 1000);
}

/** 今天之前最近的一个星期 `weekday`（0 为周日……6 为周六）是几天前：1 到 7。 */
export const daysSince = (weekday: number): number => ((new Date(NOW * 1000).getDay() - weekday + 7) % 7) || 7;

const pad = (n: number) => String(n).padStart(2, '0');

/** 一个时间所在的本地日期，写成 `2026-10-12`（与网页自己的 `dayOf` 相同，这个 chunk 不导入它）。 */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/** 本地日期 `YYYY-MM-DD` 开始时的 Unix 秒数。 */
export const dayStart = (day: string): number => new Date(day + 'T00:00:00').getTime() / 1000;
/** 本地日期 `YYYY-MM-DD` 最后一秒的 Unix 秒数。 */
export const dayEnd = (day: string): number => dayStart(day) + 86_399;
