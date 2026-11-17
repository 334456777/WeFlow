// The cascade layers of the style sheets (#827): tokens.css names them in their order before any rule, main.tsx loads
// it before anything else, and every rule of a sheet is in that sheet's layer, since a rule outside every layer would
// win over all of them.
import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const src = join(import.meta.dir, '..');
const ORDER = '@layer tokens, app, pages, skin;';
/** The skins other than the default, a sheet each, named by the skin (skin.ts). */
const SKINS = 'styles/skins/';

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

/** The selectors of the style rules in `css`, wherever they are nested (in a layer, a media query); keyframes left out. */
function selectors(css: string): string[] {
  const found: string[] = [];
  let depth = 0;
  let keyframes = -1;
  let start = 0;
  for (let i = 0; i < css.length; i++) {
    if (css[i] === '{') {
      const prelude = css.slice(start, i).trim();
      if (prelude.startsWith('@keyframes') && keyframes < 0) keyframes = depth;
      // the commas between selectors, not those inside :is() and the like
      else if (!prelude.startsWith('@') && keyframes < 0) found.push(...prelude.split(/,(?![^(]*\))/).map(selector => selector.trim()));
      depth += 1;
      start = i + 1;
    } else if (css[i] === '}') {
      depth -= 1;
      if (depth === keyframes) keyframes = -1;
      start = i + 1;
    } else if (css[i] === ';') {
      start = i + 1;
    }
  }
  return found;
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

  test('main.tsx loads the default skin next, before app.css', () => {
    const imports = [...read('main.tsx').matchAll(/^import .*?'([^']+)';$/gm)].map(match => match[1]);
    expect(imports.slice(1, 3)).toEqual(['./styles/neumorphism.css', './app.css']);
  });

  test('every rule is in its sheet’s layer', () => {
    // the default skin's tokens are the ground every skin builds on, so they are tokens too
    const layer = (path: string) => path === 'styles/neumorphism.css' ? 'tokens'
      : path.startsWith(SKINS) ? 'skin' : path === 'app.css' ? 'app' : 'pages';
    const outside = sheets(src).filter(path => path !== 'styles/tokens.css')
      .filter(path => !inLayer(withoutComments(read(path)), layer(path)));
    expect(outside).toEqual([]);
  });

  test('the selectors of a sheet are read wherever they are nested', () => {
    const own = ":root[data-skin='a']";
    expect(selectors(`@layer skin { ${own} { --x: 1; } ${own} .b, ${own} :is(.c, .d) { color: red; }`
      + ` @media (width < 9px) { ${own} p { margin: 0; } } @keyframes k { from { opacity: 0; } 50% { opacity: 1; } } }`))
      .toEqual([own, `${own} .b`, `${own} :is(.c, .d)`, `${own} p`]);
  });

  test('a skin’s rules hold only while it is on (#830)', () => {
    const astray = sheets(src).filter(path => path.startsWith(SKINS)).flatMap(path => {
      const own = `:root[data-skin='${path.slice(SKINS.length, -'.css'.length)}']`;
      return selectors(withoutComments(read(path))).filter(selector => !selector.startsWith(own)).map(selector => `${path}: ${selector}`);
    });
    expect(astray).toEqual([]);
  });
});
