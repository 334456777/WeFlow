// `bun run i18n` 的检查：去掉一行里的注释，以及注释里的中文和 `t('…')` 不算页面的文字。
import { describe, expect, test } from 'bun:test';
import { codeOf, scan } from './i18n';

/** 依次去掉 `lines` 每一行的注释，块注释可以跨行。 */
function code(...lines: string[]): string[] {
  const state = { block: false };
  return lines.map(line => codeOf(line, state));
}

describe('codeOf', () => {
  test('代码后面的 // 注释不算', () => {
    expect(code('remove(); // 删除这一行')).toEqual(['remove(); ']);
  });

  test('引号里的 // 不是注释', () => {
    expect(code("const url = 'https://example.com/a'; // 链接")).toEqual(["const url = 'https://example.com/a'; "]);
    expect(code('const s = `a // b`;')).toEqual(['const s = `a // b`;']);
  });

  test('JSX 里的块注释不算，前后的文字照常保留', () => {
    expect(code('<p>{/* 次要说明 */}标题</p>')).toEqual(['<p>{}标题</p>']);
  });

  test('跨行的块注释到结束处为止都不算', () => {
    expect(code('/**', ' * 卡片的说明', ' */ const card = 1;')).toEqual(['', '', ' const card = 1;']);
  });

  test('转义的引号不结束字符串', () => {
    expect(code("t('它\\'s // 不是注释')")).toEqual(["t('它\\'s // 不是注释')"]);
  });
});

describe('scan', () => {
  const english = { 删除: 'Delete', 标题: 'Title' };

  test('注释里的中文和 t() 不算，页面上没放进 t() 的中文算', () => {
    const page = [
      "// 顶部的说明，提到 t('不存在的文字')",
      '/** 一张卡片。 */',
      "export const Card = () => <button>{t('删除')}</button>; // 删除按钮",
      "<h2>{/* 次要说明 */}{t('标题')}</h2>",
      '<p>还没翻译</p>',
    ].join('\n');
    const { used, missing, untranslated } = scan([['cards/Card.tsx', page]], english);
    expect([...used].sort()).toEqual(['删除', '标题']);
    expect(missing).toEqual([]);
    expect(untranslated).toEqual(new Map([['cards/Card.tsx', 1]]));
  });

  test('缺少英文的文字带上文件名和行号', () => {
    const { missing } = scan([['format.ts', "const a = 1;\nexport const label = msg('新的文字');"]], english);
    expect(missing).toEqual(['format.ts:2: 新的文字']);
  });

  test('写了 i18n-ignore 的那一行不列，i18n/ 下只看 index.ts', () => {
    const { untranslated } = scan(
      [
        ['payments/labels.ts', "export const PAID = '已支付'; // i18n-ignore: 微信支付消息里的标签"],
        ['i18n/zh.ts', "export const zh = { 删除: '删除' };"],
        ['i18n/index.ts', "export const fallback = '中文';"],
      ],
      english,
    );
    expect(untranslated).toEqual(new Map([['i18n/index.ts', 1]]));
  });
});
