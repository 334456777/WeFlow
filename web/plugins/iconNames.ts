// Phosphor names each icon component for React DevTools (`GearIcon`), about 1 kB of the first screen's gzip (#783).
// Nothing on the page reads the names, so the production build drops them.
import type { Plugin } from 'vite';

/** `code` (one of Phosphor's component modules) without its `displayName` assignments. */
export function dropDisplayNames(code: string): string {
  return code.replace(/^\w+\.displayName = "\w+";\r?\n/gm, '');
}

export function iconNames(): Plugin {
  return {
    name: 'weflow-icon-names',
    apply: 'build',
    transform(code, id) {
      if (!/@phosphor-icons[\\/]react[\\/]dist[\\/](?:csr|lib)[\\/]\w+\.es\.js$/.test(id)) return null;
      const trimmed = dropDisplayNames(code);
      return trimmed === code ? null : { code: trimmed, map: null };
    },
  };
}
