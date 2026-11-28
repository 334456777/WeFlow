import { describe, expect, it } from '#test';
import { setLang } from '../i18n';
import type { ParsedMessage } from '../api/messages';
import { formatDayHeading, formatDuration, formatFileSize } from '../format';
import { appLabel, withoutLabel } from './messages';

const message = (localType: number, extra: Partial<ParsedMessage> = {}): ParsedMessage =>
  ({ _cursor: 'c', localId: 1, serverId: '1', localType, createTime: 0, isSend: 0, senderUsername: 'a', parsedContent: '', ...extra });
const app = (sub: number, kind: string, extra: Partial<ParsedMessage> = {}) => message(sub * 4_294_967_296 + 49, { appMsgKind: kind, ...extra });

describe('appLabel', () => {
  it('labels app cards by their source', () => {
    expect(appLabel(app(5, 'official-link', { appMsgSourceName: '设计观察' }))).toBe('设计观察');
    expect(appLabel(app(33, 'miniapp'))).toBe('小程序');
    expect(appLabel(app(5, 'link', { appMsgAppName: '哔哩哔哩' }))).toBe('哔哩哔哩');
  });
});

describe('formatting', () => {
  it('drops a leading label, and writes sizes, durations and day headings', async () => {
    expect(withoutLabel('[文件] 旅行计划.pdf')).toBe('旅行计划.pdf');
    expect(withoutLabel('[呲牙] 早')).toBe('早');
    expect(withoutLabel('早[呲牙]')).toBe('早[呲牙]');
    expect([0, 900, 2_483_200, 5 * 1024 ** 3].map(formatFileSize)).toEqual(['', '900 B', '2.4 MB', '5.0 GB']);
    expect([7, 42, 75].map(formatDuration)).toEqual(['7″', '42″', '1′15″']);
    const now = new Date('2026-10-16T10:00:00');
    expect(['2026-10-16', '2026-10-15', '2026-10-12', '2025-12-31'].map(day => formatDayHeading(day, now)))
      .toEqual(['今天', '昨天', '10月12日 周一', '2025年12月31日 周三']);
    await setLang('en');
    try {
      expect(['2026-10-16', '2026-10-15', '2026-10-12', '2025-12-31'].map(day => formatDayHeading(day, now)))
        .toEqual(['Today', 'Yesterday', 'Mon, Oct 12', 'Wed, Dec 31, 2025']);
    } finally {
      setLang('zh');
    }
  });
});
