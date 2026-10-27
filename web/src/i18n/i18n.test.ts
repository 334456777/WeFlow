import { afterEach, describe, expect, test } from 'bun:test';
import { lang, setLang, t } from '.';
import { en } from './en';

afterEach(() => setLang('zh'));

describe('the page speaks the server\'s language (#439)', () => {
  test('Chinese as written, English from en.ts, and the Chinese where it has none', () => {
    expect(t('任务中心')).toBe('任务中心');
    setLang('en');
    expect(t('任务中心')).toBe(en['任务中心']);
    expect(t('一段 en.ts 里没有的话')).toBe('一段 en.ts 里没有的话');
  });

  test('names are filled in either language', () => {
    expect(t('回复不是 JSON（HTTP {status}）', { status: 502 })).toBe('回复不是 JSON（HTTP 502）');
    setLang('en');
    expect(t('回复不是 JSON（HTTP {status}）', { status: 502 })).toBe('The answer is not JSON (HTTP 502)');
  });

  test('the document says it, and the next visit starts in it', () => {
    setLang('en');
    expect(document.documentElement.lang).toBe('en');
    expect(localStorage.getItem('weflow.lang')).toBe('en');
    setLang('zh');
    expect(document.documentElement.lang).toBe('zh-CN');
  });

  test('what an older server or anything else says leaves it as it is', () => {
    setLang('en');
    setLang(undefined);
    setLang('fr');
    expect(lang()).toBe('en');
  });

  test('an English text keeps the names of its Chinese', () => {
    const names = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort();
    for (const [zh, english] of Object.entries(en)) expect(names(english)).toEqual(names(zh));
  });
});
