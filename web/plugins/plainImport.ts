// Vite wraps each import() in a helper that first preloads what the imported chunk needs, its own imports and CSS
// (#783). The page's import()s load chunks with neither: the English (src/i18n), the sample server (src/mock, which
// imports only types of the page's code) and the style sheets of the skins (src/skin.ts, as text), so the helper would
// only call the import, for about 0.6 kB of the first screen's gzip. The build puts in a helper that only calls it,
// and fails when a chunk loaded with import() has something to preload, or when Vite's helper is no longer the module
// replaced here.
import type { Plugin, Rolldown } from 'vite';

/** The module Vite takes the helper from. */
export const PRELOAD_HELPER = '\0vite/preload-helper.js';

/** Chunks loaded with import() that import other chunks or CSS: what the plain helper would not preload. */
export function needsPreload(bundle: Rolldown.OutputBundle): string[] {
  const chunks = Object.values(bundle).flatMap(file => (file.type === 'chunk' ? [file] : []));
  const loaded = new Set(chunks.flatMap(chunk => chunk.dynamicImports));
  return chunks
    .filter(chunk => loaded.has(chunk.fileName) && (chunk.imports.length > 0 || (chunk.viteMetadata?.importedCss.size ?? 0) > 0))
    .map(chunk => chunk.fileName);
}

export function plainImport(): Plugin {
  let replaced = false;
  return {
    name: 'weflow-plain-import',
    apply: 'build',
    enforce: 'pre',
    load(id) {
      if (id !== PRELOAD_HELPER) return null;
      replaced = true;
      return 'export const __vitePreload = load => load();';
    },
    generateBundle(_, bundle) {
      const dynamic = Object.values(bundle).some(file => file.type === 'chunk' && file.dynamicImports.length > 0);
      if (dynamic && !replaced) this.error('Vite no longer takes its preload helper from ' + JSON.stringify(PRELOAD_HELPER) + ': update plugins/plainImport.ts');
      const needs = needsPreload(bundle);
      if (needs.length) this.error(needs.join(', ') + ' has imports or CSS to preload: remove plugins/plainImport.ts');
    },
  };
}
