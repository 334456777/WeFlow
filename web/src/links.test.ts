import { describe, expect, it } from '#test';
import { safeUrl } from './links';

describe('safeUrl (#818)', () => {
  it('lets web pages through as they are', () => {
    for (const url of ['https://mp.weixin.qq.com/s?x=1', 'http://example.com', 'HTTPS://EXAMPLE.COM/a']) expect(safeUrl(url)).toBe(url);
  });

  it('leaves out every other address', () => {
    for (const url of ['javascript:alert(1)', 'JavaScript:alert(1)', ' javascript:alert(1)', 'data:text/html,<p>', 'file:///C:/x',
      'weixin://dl/chat', 'vbscript:x', '//example.com', '/ui/chat', '', null, undefined]) expect(safeUrl(url)).toBeUndefined();
  });
});
