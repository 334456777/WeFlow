import type { Rolldown } from 'vite';
import { describe, expect, it } from '#test';
import { needsPreload } from './plainImport';

type Chunk = { fileName: string; imports?: string[]; dynamicImports?: string[]; css?: string[] };

const bundleOf = (chunks: Chunk[]) => Object.fromEntries(chunks.map(c => [c.fileName, {
  type: 'chunk', fileName: c.fileName, imports: c.imports ?? [], dynamicImports: c.dynamicImports ?? [],
  viteMetadata: { importedCss: new Set(c.css ?? []), importedAssets: new Set() },
}])) as unknown as Rolldown.OutputBundle;

describe('the plain import() helper (#783)', () => {
  it('passes a chunk loaded with import() that needs nothing else', () => {
    expect(needsPreload(bundleOf([{ fileName: 'index.js', dynamicImports: ['en.js'] }, { fileName: 'en.js' }]))).toEqual([]);
  });

  it('names a chunk loaded with import() that imports a chunk or CSS', () => {
    expect(needsPreload(bundleOf([
      { fileName: 'index.js', dynamicImports: ['a.js', 'b.js'], imports: ['shared.js'] },
      { fileName: 'shared.js' },
      { fileName: 'a.js', imports: ['shared.js'] },
      { fileName: 'b.js', css: ['b.css'] },
    ]))).toEqual(['a.js', 'b.js']);
  });
});
