import { join } from 'node:path';
import { describe, expect, it } from '#test';
import { dropWeights, ICON_WEIGHTS } from './iconWeights';

const web = join(import.meta.dir, '..');
const defs = join(web, 'node_modules/@phosphor-icons/react/dist/defs');
const icon = (name: string) => Bun.file(join(defs, name)).text();

/** The weights of an icon's definition, run with a stand-in for React. */
function weightsOf(code: string): string[] {
  const body = code
    .replace(/^import \* as (\w+) from "react";/m, 'const $1 = { createElement: (...args) => args, Fragment: "fragment" };')
    .replace(/^export \{\s*(\w+) as default\s*\};?/m, 'return $1;');
  return [...(new Function(body)() as Map<string, unknown>).keys()];
}

describe('the icon weights (#159)', () => {
  it('keeps the weights the app draws, and the definition still runs', async () => {
    for (const name of ['Images.es.js', 'ChatCircleDots.es.js', 'Warning.es.js', 'CaretDown.es.js']) {
      const code = await icon(name);
      expect(weightsOf(code)).toEqual(['bold', 'duotone', 'fill', 'light', 'regular', 'thin']);
      const trimmed = dropWeights(code, ICON_WEIGHTS)!;
      expect(weightsOf(trimmed)).toEqual(['bold', 'fill', 'regular']);
      expect(trimmed.length).toBeLessThan(code.length * 0.7);
    }
    expect(dropWeights('export const x = 1;', ICON_WEIGHTS)).toBeNull();
    const images = await icon('Images.es.js');
    expect(() => dropWeights(images, ['heavy'])).toThrow();
  });

  it('is given every weight the app asks for', async () => {
    const asked = new Set<string>();
    const src = join(web, 'src');
    for await (const path of new Bun.Glob('**/*.{ts,tsx}').scan(src)) {
      if (/\.test\.tsx?$/.test(path)) continue;
      for (const [, value] of (await Bun.file(join(src, path)).text()).matchAll(/\bweight=(\{[^}]*\}|"[^"]*")/g)) {
        for (const [, weight] of value.matchAll(/['"](\w+)['"]/g)) asked.add(weight);
      }
    }
    expect(asked.size).toBeGreaterThan(0);
    expect([...asked].filter(w => !ICON_WEIGHTS.includes(w))).toEqual([]);
  });
});
