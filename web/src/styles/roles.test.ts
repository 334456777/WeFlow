// The tokens a skin gives (#828, styles/tokens.css): the pages read the roles only, never a skin's raw tokens, and
// every variable read is given somewhere, so a misspelt role shows here instead of as a surface drawn without it.
import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const src = join(import.meta.dir, '..');
const SKIN = 'styles/neumorphism.css';
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
const sheets = all.filter(path => path.endsWith('.css'));
const scripts = all.filter(path => !path.endsWith('.css'));
const names = (text: string, pattern: RegExp) => new Set([...text.matchAll(pattern)].map(match => match[1]));
/** Variables a text reads with var(). */
const reads = (text: string) => names(text, /var\((--[\w-]+)/g);
/** Variables a style sheet sets. */
const declared = (css: string) => names(css, /(--[\w-]+)\s*:/g);

describe('the skin’s tokens', () => {
  test('only the skin reads or sets its raw tokens', () => {
    const using = all.filter(path => path !== SKIN && /--neu-[\w-]/.test(read(path)));
    expect(using).toEqual([]);
  });

  test('every variable read is given by a style sheet or set by a script', () => {
    const given = new Set(sheets.flatMap(path => [...declared(read(path))]));
    // set from a script: element.style.setProperty('--x', …) and the like
    for (const path of scripts) for (const name of names(read(path), /['"`](--[\w-]+)['"`]/g)) given.add(name);
    const missing = all.flatMap(path => [...reads(read(path))].filter(name => !given.has(name)).map(name => `${path}: ${name}`));
    expect(missing).toEqual([]);
  });

  test('every role the skin gives is read somewhere else', () => {
    const skin = read(SKIN);
    const roles = [...declared(skin)].filter(name => !name.startsWith('--neu-') && !RESERVED.includes(name));
    const elsewhere = new Set(all.filter(path => path !== SKIN).flatMap(path => [...reads(read(path))]));
    expect(roles.filter(name => !elsewhere.has(name))).toEqual([]);
  });
});
