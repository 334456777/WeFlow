import { join } from 'node:path';
import { describe, expect, it } from '#test';
import { dropWeights, ICON_WEIGHTS, iconUses, readIconSources } from './iconWeights';

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

  it('reads the weights each icon is drawn with', () => {
    const drawn = (text: string) => iconUses([{ text }]);
    expect(drawn(`import { Check } from '@phosphor-icons/react';\nexport const x = <Check weight="bold" />;`).get('Check')).toEqual(['bold']);
    expect(drawn(`import { CircleNotch } from '@phosphor-icons/react';\nexport const x = <CircleNotch size={16} className="spin" />;`).get('CircleNotch')).toEqual(['regular']);
    expect(drawn(`import { Play } from '@phosphor-icons/react';\nexport const x = <Play weight={on ? 'fill' : 'regular'} />;`).get('Play')).toEqual(['regular', 'fill']);
    expect(drawn(`import { Gear } from '@phosphor-icons/react';
      const navigation = [{ icon: Gear }];
      export const view = navigation.map(item => { const Icon = item.icon; return <Icon weight={current ? 'fill' : 'regular'} />; });
    `).get('Gear')).toEqual(['regular', 'fill']);
    expect(drawn(`import { Plugs } from '@phosphor-icons/react';
      const sections = [{ icon: Plugs }];
      export const view = sections.map(({ icon: Glyph }) => <Glyph size={18} />);
    `).get('Plugs')).toEqual(['regular']);
    expect(drawn(`import { Archive } from '@phosphor-icons/react';\nexport const view = <ScrollPage label="备份" icon={Archive} />;`).get('Archive')).toEqual(['regular']);
    expect(drawn(`import { Key } from '@phosphor-icons/react';\nexport const x = <span>API Key</span>;\nexport const y = <Key size={16} />;`).get('Key')).toEqual(['regular']);
    const passed = drawn(`import { Archive } from '@phosphor-icons/react';\nexport const view = host(Archive);`);
    expect(passed.get('Archive')).toEqual(['regular', 'fill', 'bold']);
    // an arrow in the attributes is not the end of the tag, so the weight after it still counts
    expect(drawn(`import { Play } from '@phosphor-icons/react';
      export const x = <Play onClick={() => start()} weight="fill" />;
    `).get('Play')).toEqual(['fill']);
    expect(drawn(`import { Play } from '@phosphor-icons/react';
      export const x = <Play
        onClick={() => start()}
        weight="fill"
      />;
    `).get('Play')).toEqual(['fill']);
    // a spread, or a tag that never ends, can draw any weight
    expect(drawn(`import { Play } from '@phosphor-icons/react';\nexport const x = <Play {...props} />;`).get('Play')).toEqual(['regular', 'fill', 'bold']);
    expect(drawn(`import { Play } from '@phosphor-icons/react';\nexport const x = <Play weight="fill"`).get('Play')).toEqual(['regular', 'fill', 'bold']);
    expect(drawn(`import { Gear } from '@phosphor-icons/react';
      const items = [{ icon: Gear }];
      export const view = items.map(item => { const Icon = item.icon; return <Icon onClick={() => open()} weight={current ? 'fill' : 'regular'} />; });
    `).get('Gear')).toEqual(['regular', 'fill']);
  });

  it('keeps only the weights the pages draw', async () => {
    const uses = iconUses(readIconSources(join(web, 'src')));
    expect(uses.get('CircleNotch')).toEqual(['regular']);
    expect(uses.get('Check')).toEqual(['bold']);
    expect(uses.get('ChatCircleDots')).toEqual(['regular', 'fill']);
    expect(uses.get('DotsThreeOutline')).toEqual(['regular', 'fill']);
    expect(uses.get('Gear')).toEqual(['regular', 'fill']);
    expect(uses.get('SidebarSimple')).toEqual(['regular']);
    expect(uses.get('Key')).toEqual(['regular']);
    const gear = dropWeights(await icon('Gear.es.js'), uses.get('Gear')!)!;
    expect(weightsOf(gear)).toEqual(['fill', 'regular']);
    expect(gear.length).toBeLessThan(dropWeights(await icon('Gear.es.js'), ICON_WEIGHTS)!.length);
    for (const [name, weights] of uses) {
      expect(weights.every(weight => ICON_WEIGHTS.includes(weight))).toBe(true);
      expect(weights.length).toBeGreaterThan(0);
      const code = await icon(name + '.es.js');
      expect(weightsOf(dropWeights(code, weights)!)).toEqual([...weights].sort((a, b) => weightsOf(code).indexOf(a) - weightsOf(code).indexOf(b)));
    }
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
