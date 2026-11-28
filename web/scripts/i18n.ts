// 网页的英文（#439）：src/ 中每个 `t('…')`、`msg('…')` 的文字都要在 src/i18n/en.ts 里有英文。`bun run i18n` 按文件列出
// 缺少英文的文字、没有文字再用到的英文，以及页面上还没放进 `t()` 的中文；`bun run i18n --check` 在有前两种时失败（CI 运行
// 它）。属于数据而不是页面用语的中文（如读到的微信支付消息里的标签）在那一行写 `// i18n-ignore: <原因>`，不列进清单；注释
// 里的中文和注释里提到的 `t('…')` 都不算。
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { en } from '../src/i18n/en';

const SRC = fileURLToPath(new URL('../src', import.meta.url));
const HAN = /[一-鿿]/;
// 紧跟在 `t(` 后面、写在引号里的文字，与页面的写法一致：只占一行，不含 `${}`
const CALL = /\b(?:t|msg)\(\s*(['"`])((?:\\.|(?!\1)[^\\\n])*)\1/g;
const IGNORE = /\/\/ i18n-ignore\b/;
const LITERAL = /(['"`])((?:\\.|(?!\1)[^\\\n])*)\1|>([^<>{}\n]+)</g;

/** 要检查的页面源文件：src/ 下的 `.ts`、`.tsx`，不含示例服务端、测试、夹具、生成的契约类型和英文表本身。 */
function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'mock' ? [] : files(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.|\.fixture\.|contract\.gen|i18n[\\/]en\.ts$/.test(path) ? [path] : [];
  });
}

const unquote = (quote: string, body: string) => (quote === '`' ? body : JSON.parse('"' + body.replace(/\\'/g, "'").replace(/"/g, '\\"') + '"'));

/**
 * 一行去掉注释后剩下的代码：`//` 之后的部分和块注释（包括 JSX 里的块注释）不算，引号里的不算注释。`state.block` 记录上一行的块注释
 * 是否还没结束；字符串跨行（多行的模板字符串）时按每行重新开始计算引号。
 */
export function codeOf(line: string, state: { block: boolean }): string {
  let code = '';
  let quote = '';
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (state.block) {
      if (c === '*' && line[i + 1] === '/') {
        state.block = false;
        i++;
      }
    } else if (quote) {
      code += c;
      if (c === '\\') code += line[++i] ?? '';
      else if (c === quote) quote = '';
    } else if (c === '/' && line[i + 1] === '/') {
      break;
    } else if (c === '/' && line[i + 1] === '*') {
      state.block = true;
      i++;
    } else {
      if (c === "'" || c === '"' || c === '`') quote = c;
      code += c;
    }
  }
  return code;
}

/** 检查的结果：用到的文字、缺少英文的文字（`文件:行: 文字`），以及每个文件里还没放进 `t()` 的中文的个数。 */
export interface Scan {
  used: Set<string>;
  missing: string[];
  untranslated: Map<string, number>;
}

/** 按 `english` 检查 `sources`（相对 src/ 的文件名和内容）。`i18n/` 下只看 `index.ts`，其余是各语言的表。 */
export function scan(sources: Iterable<[name: string, text: string]>, english: Record<string, string>): Scan {
  const result: Scan = { used: new Set(), missing: [], untranslated: new Map() };
  for (const [name, text] of sources) {
    if (name.startsWith('i18n/') && name !== 'i18n/index.ts') continue;
    const state = { block: false };
    for (const [i, line] of text.split('\n').entries()) {
      const code = codeOf(line, state);
      if (!code.trim()) continue;
      const inCalls = new Set<number>();
      for (const m of code.matchAll(CALL)) {
        const words = unquote(m[1], m[2]);
        result.used.add(words);
        if (!(words in english)) result.missing.push(`${name}:${i + 1}: ${words}`);
        inCalls.add(m.index! + m[0].indexOf(m[1]));
      }
      if (IGNORE.test(line)) continue;
      for (const m of code.matchAll(LITERAL)) {
        const body = m[2] ?? m[3];
        if (!HAN.test(body)) continue;
        const at = m.index! + (m[3] === undefined ? 0 : 1);
        if (m[3] === undefined && inCalls.has(at)) continue;
        result.untranslated.set(name, (result.untranslated.get(name) ?? 0) + 1);
      }
    }
  }
  return result;
}

if (import.meta.main) {
  const sources = files(SRC).map((path): [string, string] => [relative(SRC, path).replace(/\\/g, '/'), readFileSync(path, 'utf8')]);
  const { used, missing, untranslated } = scan(sources, en);
  const stale = Object.keys(en).filter(text => !used.has(text));
  const total = [...untranslated.values()].reduce((a, b) => a + b, 0);
  if (untranslated.size) {
    console.log(`Chinese not in a t() yet: ${total} in ${untranslated.size} files`);
    for (const [name, n] of [...untranslated].sort((a, b) => b[1] - a[1])) console.log(`  ${n}\t${name}`);
  }
  for (const line of missing) console.error('no English: ' + line);
  for (const text of stale) console.error('English no text uses: ' + text);
  if (process.argv.includes('--check') && (missing.length || stale.length)) process.exit(1);
}
