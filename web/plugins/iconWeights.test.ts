import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { dropWeights, ICON_WEIGHTS } from './iconWeights';

const web = join(__dirname, '..');
const defs = join(web, 'node_modules/@phosphor-icons/react/dist/defs');

/** The weights of an icon's definition, run with a stand-in for React. */
function weightsOf(code: string): string[] {
  const body = code
    .replace(/^import \* as (\w+) from "react";/m, 'const $1 = { createElement: (...args) => args, Fragment: "fragment" };')
    .replace(/^export \{\s*(\w+) as default\s*\};?/m, 'return $1;');
  return [...(new Function(body)() as Map<string, unknown>).keys()];
}

describe('the icon weights (#159)', () => {
  it('keeps the weights the app draws, and the definition still runs', () => {
    for (const name of ['Images.es.js', 'ChatCircleDots.es.js', 'Warning.es.js', 'CaretDown.es.js']) {
      const code = readFileSync(join(defs, name), 'utf8');
      expect(weightsOf(code)).toEqual(['bold', 'duotone', 'fill', 'light', 'regular', 'thin']);
      const trimmed = dropWeights(code, ICON_WEIGHTS)!;
      expect(weightsOf(trimmed)).toEqual(['bold', 'fill', 'regular']);
      expect(trimmed.length).toBeLessThan(code.length * 0.7);
    }
    expect(dropWeights('export const x = 1;', ICON_WEIGHTS)).toBeNull();
    expect(() => dropWeights(readFileSync(join(defs, 'Images.es.js'), 'utf8'), ['heavy'])).toThrow();
  });

  it('is given every weight the app asks for', () => {
    const asked = new Set<string>();
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) walk(path);
        else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
          for (const [, value] of readFileSync(path, 'utf8').matchAll(/\bweight=(\{[^}]*\}|"[^"]*")/g)) {
            for (const [, weight] of value.matchAll(/['"](\w+)['"]/g)) asked.add(weight);
          }
        }
      }
    };
    walk(join(web, 'src'));
    expect(asked.size).toBeGreaterThan(0);
    expect([...asked].filter(w => !ICON_WEIGHTS.includes(w))).toEqual([]);
  });
});
