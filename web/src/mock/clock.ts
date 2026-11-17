// The sample's times, counted back from when the page was opened: its records are always recent ones.

/** Unix seconds when the sample was loaded. */
export const NOW = Math.floor(Date.now() / 1000);

/** Unix seconds `days`, `hours` and `minutes` before the page was opened. */
export const ago = (days: number, hours = 0, minutes = 0): number => NOW - ((days * 24 + hours) * 60 + minutes) * 60;

const pad = (n: number) => String(n).padStart(2, '0');

/** The local day of a time, as `2026-10-12` (as the page's own `dayOf`, which this chunk does not import). */
export function dayOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.getFullYear() + '-' + pad(date.getMonth() + 1) + '-' + pad(date.getDate());
}

/** Unix seconds of the start of a local day `YYYY-MM-DD`, and of its end. */
export const dayStart = (day: string): number => new Date(day + 'T00:00:00').getTime() / 1000;
export const dayEnd = (day: string): number => dayStart(day) + 86_399;
