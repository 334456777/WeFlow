import { describe, expect, it } from '#test';
import { baseType, type ParsedMessage } from './api/messages';
import { kindOf } from './messageKind';

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

  it('draws a pat as a system line, not a card (#1200)', () => {
    expect(kindOf(app(62, 'card', { parsedContent: '"A" 拍了拍 "B"' }))).toBe('system');
  });
});
