// Every export of src/ has a JSDoc comment (`/** … */`) right above it, saying what it is and when it returns or shows
// what (#915). `bun run jsdoc` lists the exports without one by file and line and fails if there is any (CI runs it).
// Tests, fixtures and the generated api/contract.gen.ts are left out, and so is a re-export (`export { … } from`,
// `export type { … }`): its item has the comment where it is declared. The check reads lines, as the TypeScript package
// has no syntax tree for scripts: an export is a line that starts with `export`, so a declaration nested in another
// (inside a `declare module`, say) is not one.
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../src', import.meta.url));
// a declaration exported at the top level, not a list of names
const EXPORT = /^export\s+(?!\{|\*|type\s*\{)/;

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return files(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.|\.fixture\.|contract\.gen\.ts$/.test(entry.name) ? [path] : [];
  });
}

/** Whether the lines right above `at` end a comment that starts with `/**`. */
function documented(lines: string[], at: number): boolean {
  if (at === 0 || !lines[at - 1].trimEnd().endsWith('*/')) return false;
  for (let i = at - 1; i >= 0; i--) {
    const start = lines[i].indexOf('/*');
    if (start >= 0) return lines[i].startsWith('/**', start);
  }
  return false;
}

const bare: string[] = [];
for (const path of files(SRC)) {
  const name = relative(SRC, path).replace(/\\/g, '/');
  const lines = readFileSync(path, 'utf8').split(/\r?\n/);
  for (const [i, line] of lines.entries()) {
    if (EXPORT.test(line) && !documented(lines, i)) bare.push(`${name}:${i + 1}: ${line.trim()}`);
  }
}

for (const line of bare.sort()) console.error('no JSDoc: ' + line);
if (bare.length) process.exit(1);
