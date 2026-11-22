import { describe, expect, it, resetZone } from '#test';

describe('resetZone (#1087)', () => {
  it('puts the process back in the time zone it started in', () => {
    const zone = () => Intl.DateTimeFormat().resolvedOptions().timeZone;
    const started = zone();
    const offset = new Date(2026, 0, 1).getTimezoneOffset();
    // a zone other than the one the tests run in, whichever that is
    process.env.TZ = offset === 480 ? 'Asia/Tokyo' : 'America/Los_Angeles';
    expect(new Date(2026, 0, 1).getTimezoneOffset()).not.toBe(offset);
    resetZone();
    expect(zone()).toBe(started);
    expect(new Date(2026, 0, 1).getTimezoneOffset()).toBe(offset);
  });
});
