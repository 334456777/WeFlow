// The cascade layers of the style sheets (#827): tokens.css names them in their order before any rule, main.tsx loads
// it before anything else, and every rule of a sheet is in that sheet's layer, since a rule outside every layer would
// win over all of them.
import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const src = join(import.meta.dir, '..');
const ORDER = '@layer tokens, app, pages, skin;';

const read = (path: string) => readFileSync(join(src, path), 'utf8');
const withoutComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '').trim();

function sheets(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? sheets(join(dir, entry.name))
    : entry.name.endsWith('.css') ? [relative(src, join(dir, entry.name)).replaceAll('\\', '/')] : []);
}

/** Whether `css` is one `@layer <name> { … }` block and nothing else. */
function inLayer(css: string, name: string): boolean {
  const head = `@layer ${name} {`;
  if (!css.startsWith(head) || !css.endsWith('}')) return false;
  let depth = 0;
  for (let i = head.length - 1; i < css.length; i++) {
    if (css[i] === '{') depth += 1;
    else if (css[i] === '}' && --depth === 0) return i === css.length - 1;
  }
  return false;
}

describe('cascade layers', () => {
  test('tokens.css names the layers first', () => {
    const css = withoutComments(read('styles/tokens.css'));
    expect(css.startsWith(ORDER)).toBe(true);
    expect(inLayer(css.slice(ORDER.length).trim(), 'tokens')).toBe(true);
  });

  test('main.tsx loads tokens.css before any other module', () => {
    const imports = [...read('main.tsx').matchAll(/^import .*?'([^']+)';$/gm)].map(match => match[1]);
    expect(imports[0]).toBe('./styles/tokens.css');
  });

  test('every rule is in its sheet’s layer', () => {
    const outside = sheets(src).filter(path => path !== 'styles/tokens.css')
      .filter(path => !inLayer(withoutComments(read(path)), path === 'app.css' ? 'app' : 'pages'));
    expect(outside).toEqual([]);
  });
});
