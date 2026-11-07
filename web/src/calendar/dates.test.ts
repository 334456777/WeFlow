import { describe, expect, it } from '#test';
import { setLang } from '../i18n';
import { addDays, addMonths, dayLabel, endOfWeek, monthGrid, ordered, startOfWeek, weekdayOf, weekdays } from './dates';

describe('dates', () => {
  it('moves by days and months in local time, keeping the day where the month has it', () => {
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01');
    expect(addDays('2026-03-01', -1)).toBe('2026-02-28');
    expect(addMonths('2026-01-31', 1)).toBe('2026-02-28');
    expect(addMonths('2024-01-31', 1)).toBe('2024-02-29');
    expect(addMonths('2026-10-16', -12)).toBe('2025-10-16');
    expect(addMonths('2026-12-05', 1)).toBe('2027-01-05');
  });

  it('starts the week on Monday', () => {
    expect(weekdayOf('2026-10-12')).toBe(0);
    expect(weekdayOf('2026-10-18')).toBe(6);
    expect(startOfWeek('2026-10-16')).toBe('2026-10-12');
    expect(endOfWeek('2026-10-16')).toBe('2026-10-18');
    const grid = monthGrid(2026, 9);
    expect(grid).toHaveLength(6);
    expect(grid[0][0]).toBe('2026-09-28');
    expect(grid[0][3]).toBe('2026-10-01');
    expect(grid.flat()).toContain('2026-10-31');
  });

  it('names a day for a screen reader, and orders two days', () => {
    expect(dayLabel('2026-10-12')).toBe('2026年10月12日 星期一');
    expect(ordered('2026-10-12', '2026-10-03')).toEqual(['2026-10-03', '2026-10-12']);
  });

  it('names the days in the page\'s language, the week from Monday', async () => {
    expect(weekdays().map(d => d.short).join('')).toBe('一二三四五六日');
    expect(weekdays()[0].long).toBe('星期一');
    await setLang('en');
    try {
      expect(dayLabel('2026-10-12')).toBe('Monday, October 12, 2026');
      expect(weekdays().map(d => d.short).join('')).toBe('MTWTFSS');
      expect(weekdays()[6].long).toBe('Sunday');
    } finally {
      setLang('zh');
    }
  });
});
