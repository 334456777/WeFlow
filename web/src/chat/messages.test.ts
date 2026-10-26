import { describe, expect, it } from '#test';
import type { ParsedMessage } from '../api/messages';
import { appLabel, baseType, formatDayHeading, formatDuration, formatFileSize, kindOf, withoutLabel } from './messages';

const message = (localType: number, extra: Partial<ParsedMessage> = {}): ParsedMessage =>
  ({ _cursor: 'c', localId: 1, serverId: '1', localType, createTime: 0, isSend: 0, senderUsername: 'a', parsedContent: '', ...extra });
const app = (sub: number, kind: string, extra: Partial<ParsedMessage> = {}) => message(sub * 4_294_967_296 + 49, { appMsgKind: kind, ...extra });

describe('kindOf', () => {
  it('reads WeChat types, and app messages by the kind the server names', () => {
    expect(baseType(244813135921)).toBe(49);
    expect([1, 3, 34, 42, 43, 47, 48, 50, 10000, 10002, 9999].map(t => kindOf(message(t)))).toEqual(
      ['text', 'image', 'voice', 'contact', 'video', 'emoji', 'location', 'call', 'system', 'system', 'text']);
    expect([
      app(57, 'quote'), app(6, 'file'), app(5, 'link'), app(5, 'official-link'), app(51, 'finder'), app(19, 'chat-record'),
      app(2001, 'red-packet'), app(2000, 'transfer'), app(33, 'miniapp'), app(99, 'something-new'),
    ].map(kindOf)).toEqual(['quote', 'file', 'link', 'link', 'finder', 'record', 'money', 'money', 'link', 'text']);
  });

  it('labels app cards by their source', () => {
    expect(appLabel(app(5, 'official-link', { appMsgSourceName: '设计观察' }))).toBe('设计观察');
    expect(appLabel(app(33, 'miniapp'))).toBe('小程序');
    expect(appLabel(app(5, 'link', { appMsgAppName: '哔哩哔哩' }))).toBe('哔哩哔哩');
  });
});

describe('formatting', () => {
  it('drops a leading label, and writes sizes, durations and day headings', () => {
    expect(withoutLabel('[文件] 旅行计划.pdf')).toBe('旅行计划.pdf');
    expect(withoutLabel('[呲牙] 早')).toBe('早');
    expect(withoutLabel('早[呲牙]')).toBe('早[呲牙]');
    expect([0, 900, 2_483_200, 5 * 1024 ** 3].map(formatFileSize)).toEqual(['', '900 B', '2.4 MB', '5.0 GB']);
    expect([7, 42, 75].map(formatDuration)).toEqual(['7″', '42″', '1′15″']);
    const now = new Date('2026-10-16T10:00:00');
    expect(['2026-10-16', '2026-10-15', '2026-10-12', '2025-12-31'].map(day => formatDayHeading(day, now)))
      .toEqual(['今天', '昨天', '10月12日 周一', '2025年12月31日 周三']);
  });
});
