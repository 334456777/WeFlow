// Phosphor icons carry the paths of six weights each, though the app draws three (#159): the build keeps those and
// drops the others from each icon's definition, `new Map([["bold", …], ["duotone", …], …])`.
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

export function iconWeights(keep: readonly string[] = ICON_WEIGHTS): Plugin {
  return {
    name: 'weflow-icon-weights',
    transform(code, id) {
      if (!/@phosphor-icons[\\/]react[\\/]dist[\\/]defs[\\/]/.test(id)) return null;
      const trimmed = dropWeights(code, keep);
      return trimmed === null ? null : { code: trimmed, map: null };
    },
  };
}
