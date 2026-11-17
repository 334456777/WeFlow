// The sample's times, counted back from when the page was opened: its records are always recent ones.

/** Unix seconds when the sample was loaded. */
export const NOW = Math.floor(Date.now() / 1000);

/** Unix seconds `days`, `hours` and `minutes` before the page was opened. */
export const ago = (days: number, hours = 0, minutes = 0): number => NOW - ((days * 24 + hours) * 60 + minutes) * 60;

/**
 * Unix seconds of `days` days before today at the clock time `HH:MM`: for what is said at a time of day (good morning,
 * late at night), on a day before today, so that it is always in the past.
 */
export function at(days: number, clock: string): number {
  const [hours, minutes] = clock.split(':').map(Number);
  const date = new Date(NOW * 1000);
  date.setDate(date.getDate() - days);
  date.setHours(hours, minutes, 0, 0);
  return Math.floor(date.getTime() / 1000);
}

/** How many days ago the last `weekday` before today was (0 Sunday … 6 Saturday): 1 to 7. */
export const daysSince = (weekday: number): number => ((new Date(NOW * 1000).getDay() - weekday + 7) % 7) || 7;

const pad = (n: number) => String(n).padStart(2, '0');

/** The local day of a time, as `2026-10-12` (as the page's own `dayOf`, which this chunk does not import). */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/** Unix seconds of the start of a local day `YYYY-MM-DD`, and of its end. */
export const dayStart = (day: string): number => new Date(day + 'T00:00:00').getTime() / 1000;
export const dayEnd = (day: string): number => dayStart(day) + 86_399;
