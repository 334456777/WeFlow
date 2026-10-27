// The page's English (#439): every text of a `t('…')` or `msg('…')` in src/ needs its English in src/i18n/en.ts. `bun run i18n`
// lists the texts without it, the English no text uses any more, and the Chinese of the page not in a `t()` yet, by
// file; `bun run i18n --check` fails on the first two (CI runs it).
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { en } from '../src/i18n/en';

const SRC = fileURLToPath(new URL('../src', import.meta.url));
const HAN = /[一-鿿]/;
// a text in quotes right after `t(`, as the page writes them: one line, no `${}`
const CALL = /\b(?:t|msg)\(\s*(['"`])((?:\\.|(?!\1)[^\\\n])*)\1/g;
const LITERAL = /(['"`])((?:\\.|(?!\1)[^\\\n])*)\1|>([^<>{}\n]+)</g;

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'mock' ? [] : files(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.|\.fixture\.|contract\.gen|i18n[\\/]en\.ts$/.test(path) ? [path] : [];
  });
}

const unquote = (quote: string, body: string) => (quote === '`' ? body : JSON.parse('"' + body.replace(/\\'/g, "'").replace(/"/g, '\\"') + '"'));

const used = new Set<string>();
const missing: string[] = [];
const untranslated = new Map<string, number>();
for (const path of files(SRC)) {
  const name = relative(SRC, path).replace(/\\/g, '/');
  if (name.startsWith('i18n/') && name !== 'i18n/index.ts') continue;
  const lines = readFileSync(path, 'utf8').split('\n');
  for (const [i, line] of lines.entries()) {
    const code = line.trim();
    if (code.startsWith('//') || code.startsWith('*') || code.startsWith('/*')) continue;
    const inCalls = new Set<number>();
    for (const m of line.matchAll(CALL)) {
      const text = unquote(m[1], m[2]);
      used.add(text);
      if (!(text in en)) missing.push(`${name}:${i + 1}: ${text}`);
      inCalls.add(m.index! + m[0].indexOf(m[1]));
    }
    for (const m of line.matchAll(LITERAL)) {
      const body = m[2] ?? m[3];
      if (!HAN.test(body)) continue;
      const at = m.index! + (m[3] === undefined ? 0 : 1);
      if (m[3] === undefined && inCalls.has(at)) continue;
      untranslated.set(name, (untranslated.get(name) ?? 0) + 1);
    }
  }
}
const stale = Object.keys(en).filter(text => !used.has(text));

const total = [...untranslated.values()].reduce((a, b) => a + b, 0);
if (untranslated.size) {
  console.log(`Chinese not in a t() yet: ${total} in ${untranslated.size} files`);
  for (const [name, n] of [...untranslated].sort((a, b) => b[1] - a[1])) console.log(`  ${n}\t${name}`);
}
for (const line of missing) console.error('no English: ' + line);
for (const text of stale) console.error('English no text uses: ' + text);
if (process.argv.includes('--check') && (missing.length || stale.length)) process.exit(1);
