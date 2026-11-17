// The tokens a skin gives (#828, styles/tokens.css): the pages read the roles only, never a skin's raw tokens, and
// every variable read is given somewhere, so a misspelt role shows here instead of as a surface drawn without it. A skin
// other than the default (#830, styles/skins/<id>.css) sets the tokens the default skin and the other sheets give, and
// raw tokens of its own named `--<id>-…`: a token only it gave would be missing under every other skin.
import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const src = join(import.meta.dir, '..');
/** The default skin's sheet and the prefix of its raw tokens. */
const DEFAULT = { sheet: 'styles/neumorphism.css', raw: '--neu-' };
/** The skins other than the default, a sheet each, named by the skin (skin.ts). */
const SKINS = 'styles/skins/';
/** Given for skins that show a selection in colour; neumorphism does not. */
const RESERVED = ['--accent', '--on-accent'];

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? files(join(dir, entry.name))
    : /\.(css|tsx?)$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)
      ? [relative(src, join(dir, entry.name)).replaceAll('\\', '/')] : []);
}

const read = (path: string) => readFileSync(join(src, path), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const all = files(src);
/** Every style sheet and script, path → text, comments out. */
const texts = new Map(all.map(path => [path, read(path)]));
const sheets = all.filter(path => path.endsWith('.css'));
const scripts = all.filter(path => !path.endsWith('.css'));
const names = (text: string, pattern: RegExp) => new Set([...text.matchAll(pattern)].map(match => match[1]));
/** Variables a text reads with var(). */
const reads = (text: string) => names(text, /var\((--[\w-]+)/g);
/** Variables a style sheet sets. */
const declared = (css: string) => names(css, /(--[\w-]+)\s*:/g);
/** Variables a script names: element.style.setProperty('--x', …), getPropertyValue('--x') and the like. */
const named = (script: string) => names(script, /['"`](--[\w-]+)['"`]/g);

/** The skins of `paths`, the default first: each one's sheet and the prefix of its raw tokens, `--<id>-`. */
const skinsIn = (paths: string[]) => [DEFAULT, ...paths.filter(path => path.startsWith(SKINS))
  .map(sheet => ({ sheet, raw: `--${sheet.slice(SKINS.length, -'.css'.length)}-` }))];

/** Where a skin's raw tokens are read or set outside its own sheet, in `texts`: `<path>: <prefix>…`. */
function rawAstray(texts: Map<string, string>): string[] {
  return skinsIn([...texts.keys()]).flatMap(({ sheet, raw }) => {
    const prefix = raw.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const token = new RegExp(`(?<![\\w-])${prefix}[\\w-]`);
    return [...texts].filter(([path, text]) => path !== sheet && token.test(text)).map(([path]) => `${path}: ${raw}…`);
  });
}

/** What a skin other than the default sets in `texts` that is neither given by another sheet or a script (the default
 * skin's tokens and roles, the shared tokens, those of the roles and the pages) nor a raw token of its own. */
function strayTokens(texts: Map<string, string>): string[] {
  const skins = skinsIn([...texts.keys()]).slice(1);
  const own = new Set(skins.map(skin => skin.sheet));
  const given = new Set([...texts].filter(([path]) => !own.has(path))
    .flatMap(([path, text]) => [...(path.endsWith('.css') ? declared(text) : named(text))]));
  return skins.flatMap(({ sheet, raw }) => [...declared(texts.get(sheet)!)]
    .filter(name => !given.has(name) && !name.startsWith(raw)).map(name => `${sheet}: ${name}`));
}

describe('the skin’s tokens', () => {
  test('only a skin reads or sets its raw tokens', () => {
    expect(rawAstray(texts)).toEqual([]);
  });

  test('a skin other than the default sets the tokens given elsewhere and raw tokens of its own only (#784)', () => {
    expect(strayTokens(texts)).toEqual([]);
  });

  test('the rules of raw tokens hold for a skin added', () => {
    const added = new Map([
      [DEFAULT.sheet, '@layer tokens { :root { --neu-surface: #eee; --button-bg: var(--neu-surface); --fade-bg: var(--neu-surface); } }'],
      ['app.css', '@layer app { .a { background: var(--button-bg); } .b { background: var(--paper-grain); } }'],
      ['motion.ts', "element.style.setProperty('--turn', '1');"],
      ['styles/skins/paper.css', "@layer skin { :root[data-skin='paper'] { --paper-grain: url(a.png); --button-bg: var(--paper-grain);"
        + ' --turn: 2; --grain: 1; --fade-bg: var(--neu-surface); } }'],
    ]);
    // the page reads the paper's raw token, the paper the default's; the paper gives a token no other skin has
    expect(rawAstray(added)).toEqual(['styles/skins/paper.css: --neu-…', 'app.css: --paper-…']);
    expect(strayTokens(added)).toEqual(['styles/skins/paper.css: --grain']);
  });

  test('every variable read is given by a style sheet or set by a script', () => {
    const given = new Set(sheets.flatMap(path => [...declared(texts.get(path)!)]));
    for (const path of scripts) for (const name of named(texts.get(path)!)) given.add(name);
    const missing = all.flatMap(path => [...reads(texts.get(path)!)].filter(name => !given.has(name)).map(name => `${path}: ${name}`));
    expect(missing).toEqual([]);
  });

  test('every role the default skin gives is read somewhere else', () => {
    const roles = [...declared(texts.get(DEFAULT.sheet)!)].filter(name => !name.startsWith(DEFAULT.raw) && !RESERVED.includes(name));
    // read with var(), or by a script
    const elsewhere = new Set(all.filter(path => path !== DEFAULT.sheet).flatMap(path => [...reads(texts.get(path)!)]));
    for (const path of scripts) for (const name of named(texts.get(path)!)) elsewhere.add(name);
    expect(roles.filter(name => !elsewhere.has(name))).toEqual([]);
  });
});
