// Phosphor icons carry the paths of six weights each, though the app draws three (#159): the build keeps those and
// drops the others from each icon's definition, `new Map([["bold", …], ["duotone", …], …])`. A production build
// then keeps, for each icon, only the weights that icon is actually drawn with.
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type { Plugin } from 'vite';

/** The weights the app draws; `iconWeights.test.ts` checks that it asks for no other. */
export const ICON_WEIGHTS = ['regular', 'fill', 'bold'];

/** The end of the string, comment or bracket that starts at `i` (exclusive), skipping what is inside. */
function skip(code: string, i: number): number {
  const c = code[i];
  if (c === '"' || c === "'" || c === '`') {
    for (let j = i + 1; j < code.length; j++) {
      if (code[j] === '\\') j++;
      else if (code[j] === c) return j + 1;
    }
    throw new Error('unterminated string');
  }
  if (code.startsWith('/*', i)) {
    const end = code.indexOf('*/', i + 2);
    if (end < 0) throw new Error('unterminated comment');
    return end + 2;
  }
  return i + 1;
}

/** `code` (an icon's definition) with only the `keep` weights in its map; null when it is not one. */
export function dropWeights(code: string, keep: readonly string[]): string | null {
  const open = code.indexOf('new Map([');
  if (open < 0) return null;
  const entries: { from: number; to: number; weight: string }[] = [];
  let i = open + 'new Map(['.length;
  for (;;) {
    while (i < code.length && /[\s,]/.test(code[i])) i++;
    if (code[i] === ']') break;
    if (code[i] !== '[') throw new Error('unexpected icon definition at ' + i);
    // one entry: [ "weight", element ]
    const from = i;
    let depth = 0;
    let weight = '';
    while (i < code.length) {
      const c = code[i];
      if (c === '[') { depth++; i++; }
      else if (c === ']') { depth--; i++; if (depth === 0) break; }
      else if (c === '"' && !weight) { const end = skip(code, i); weight = code.slice(i + 1, end - 1); i = end; }
      else i = skip(code, i);
    }
    entries.push({ from, to: i, weight });
  }
  const kept = entries.filter(e => keep.includes(e.weight));
  if (!kept.length) throw new Error('an icon with none of the weights kept');
  return code.slice(0, entries[0].from) + kept.map(e => code.slice(e.from, e.to)).join(',\n  ') + code.slice(entries.at(-1)!.to);
}

const WEIGHT_ATTR = /\bweight\s*=\s*(?:"([^"]*)"|'([^']*)'|\{([^}]*)\})/;

/** Weights named by one icon tag's attributes. No `weight` means the icon's own default, `regular`. */
function weightsOfTag(attrs: string): string[] {
  const found = attrs.match(WEIGHT_ATTR);
  if (!found) return ['regular'];
  if (found[1] != null || found[2] != null) return [found[1] ?? found[2]];
  const named = [...found[3].matchAll(/['"]([A-Za-z]+)['"]/g)].map(match => match[1]);
  // `weight={variable}` can be any of the three at runtime
  return named.length ? named : [...ICON_WEIGHTS];
}

function phosphorImports(text: string): Map<string, string> {
  const imported = new Map<string, string>();
  for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*['"]@phosphor-icons\/react['"]/g)) {
    for (const raw of match[1].split(',')) {
      const trimmed = raw.trim();
      if (!trimmed || trimmed.startsWith('type ')) continue;
      const spec = trimmed.match(/^(\w+)(?:\s+as\s+(\w+))?$/);
      if (!spec) continue;
      imported.set(spec[2] ?? spec[1], spec[1]);
    }
  }
  return imported;
}

/** Source with comments, quoted text and JSX text removed, so a word in a label is not a use of an icon. */
function codeOnly(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/'(?:\\.|[^'\\])*'/g, "''")
    .replace(/"(?:\\.|[^"\\])*"/g, '""')
    .replace(/`(?:\\[\s\S]|[^`\\])*`/g, '``')
    .replace(/\/\/[^\n]*/g, ' ')
    .replace(/>([^<{]*)</g, '><');
}

function addWeights(uses: Map<string, Set<string>>, icon: string, weights: readonly string[]): void {
  let have = uses.get(icon);
  if (!have) uses.set(icon, (have = new Set()));
  for (const weight of weights) have.add(weight);
}

/**
 * The weights each Phosphor icon is drawn with, by its export name (`Gear`). A tag with no `weight` counts as
 * `regular`. Icons handed to the sidebar or the phone navigation take `fill` and `regular`; a page heading takes
 * `regular`. An icon used some other way keeps every weight the app draws.
 */
export function iconUses(files: readonly { text: string }[]): Map<string, string[]> {
  const uses = new Map<string, Set<string>>();
  for (const { text } of files) {
    const imported = phosphorImports(text);
    if (!imported.size) continue;
    for (const [local, icon] of imported) {
      for (const tag of text.matchAll(new RegExp(`<${local}\\b([^>]*)>`, 'g'))) addWeights(uses, icon, weightsOfTag(tag[1]));
      let rest = text.replace(/import\s*\{[^}]+\}\s*from\s*['"]@phosphor-icons\/react['"]/g, '');
      rest = codeOnly(rest);
      rest = rest.replace(new RegExp(`<${local}\\b[^>]*>`, 'g'), '');
      rest = rest.replace(new RegExp(`\\bicon:\\s*${local}\\b`, 'g'), '');
      rest = rest.replace(new RegExp(`\\bicon=\\{${local}\\}`, 'g'), '');
      if (new RegExp(`\\b${local}\\b`).test(rest)) addWeights(uses, icon, ICON_WEIGHTS);
    }
    const stored: string[] = [];
    const aliases = new Set<string>();
    for (const match of text.matchAll(/\bicon:\s*(\w+)/g)) {
      const local = match[1];
      const icon = imported.get(local);
      if (icon) stored.push(icon);
      else aliases.add(local);
    }
    for (const match of text.matchAll(/\b(?:const|let)\s+(\w+)\s*=\s*[\w.]+\.icon\b/g)) aliases.add(match[1]);
    if (stored.length && aliases.size) {
      const drawn = new Set<string>();
      for (const alias of aliases) {
        for (const tag of text.matchAll(new RegExp(`<${alias}\\b([^>]*)>`, 'g'))) {
          for (const weight of weightsOfTag(tag[1])) drawn.add(weight);
        }
      }
      for (const icon of stored) addWeights(uses, icon, drawn.size ? [...drawn] : ICON_WEIGHTS);
    }
    for (const match of text.matchAll(/\bicon=\{(\w+)\}/g)) {
      const icon = imported.get(match[1]);
      if (!icon) continue;
      const host = text.slice(0, match.index).match(/<([A-Z]\w*)\b[^<]*$/);
      addWeights(uses, icon, host?.[1] === 'ScrollPage' ? ['regular'] : ICON_WEIGHTS);
    }
  }
  return new Map([...uses].map(([icon, weights]) => [icon, [
    ...ICON_WEIGHTS.filter(weight => weights.has(weight)),
    ...[...weights].filter(weight => !ICON_WEIGHTS.includes(weight)).sort(),
  ]]));
}

/** Source files under `root` that can draw an icon. Tests are left out: they do not ship. */
export function readIconSources(root: string): { text: string }[] {
  const out: { text: string }[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) out.push({ text: readFileSync(path, 'utf8') });
    }
  };
  walk(root);
  return out;
}

export function iconWeights(keep: readonly string[] = ICON_WEIGHTS): Plugin {
  let build = false;
  let uses: Map<string, string[]> | null = null;
  return {
    name: 'weflow-icon-weights',
    configResolved(config) {
      build = config.command === 'build';
    },
    transform(code, id) {
      const file = id.match(/@phosphor-icons[\\/]react[\\/]dist[\\/]defs[\\/](\w+)\.es\.js/);
      if (!file) return null;
      // Dev keeps every drawn weight, so changing a `weight` does not wait on this transform cache.
      let weights = keep;
      if (build && keep === ICON_WEIGHTS) {
        uses ??= iconUses(readIconSources(join(process.cwd(), 'src')));
        weights = uses.get(file[1]) ?? keep;
      }
      const trimmed = dropWeights(code, weights);
      return trimmed === null ? null : { code: trimmed, map: null };
    },
  };
}
