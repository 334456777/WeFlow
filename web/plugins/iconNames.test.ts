import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from '#test';
import { dropDisplayNames } from './iconNames';

const dist = join(import.meta.dir, '..', 'node_modules/@phosphor-icons/react/dist');

describe('the icon names (#783)', () => {
  it('drops the name of every icon component and of their base, and nothing else', async () => {
    const files = [...readdirSync(join(dist, 'csr')).filter(name => name.endsWith('.es.js')).map(name => join(dist, 'csr', name)), join(dist, 'lib/IconBase.es.js')];
    expect(files.length).toBeGreaterThan(1000);
    for (const file of files) {
      const code = await Bun.file(file).text();
      const trimmed = dropDisplayNames(code);
      expect(trimmed).not.toContain('displayName');
      const kept = new Set(trimmed.split('\n'));
      expect(code.split('\n').filter(line => !kept.has(line))).toHaveLength(1);
    }
  });
});
