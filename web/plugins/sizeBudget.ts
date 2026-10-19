// The most JavaScript the page loads before it shows (#159): the entry chunk and what it imports at once. The build
// fails above it, so a change that makes the page heavier says so and raises the budget on purpose.
import { gzipSync } from 'node:zlib';
import type { Plugin } from 'vite';

export interface Budget {
  /** Bytes, minified. */
  js: number;
  /** Bytes, gzip-compressed as served. */
  gzip: number;
}

export const BUDGET: Budget = { js: 700_000, gzip: 200_000 };

// as Vite counts them
const kb = (bytes: number) => (bytes / 1000).toFixed(1) + ' kB';

export function sizeBudget(budget: Budget = BUDGET): Plugin {
  return {
    name: 'weflow-size-budget',
    apply: 'build',
    generateBundle(_, bundle) {
      const chunks = new Map(Object.values(bundle).flatMap(f => (f.type === 'chunk' ? [[f.fileName, f] as const] : [])));
      // the entries and the chunks they import statically, once each
      const initial = new Set<string>();
      const visit = (name: string) => {
        if (initial.has(name)) return;
        initial.add(name);
        for (const next of chunks.get(name)?.imports ?? []) visit(next);
      };
      for (const chunk of chunks.values()) if (chunk.isEntry) visit(chunk.fileName);
      let js = 0;
      let gzip = 0;
      for (const name of initial) {
        const code = chunks.get(name)?.code ?? '';
        js += Buffer.byteLength(code);
        gzip += gzipSync(code, { level: 9 }).length;
      }
      const sizes = 'the page loads ' + kb(js) + ' of JavaScript (' + kb(gzip) + ' gzip) before it shows';
      if (js > budget.js || gzip > budget.gzip) {
        this.error(sizes + ', over the budget of ' + kb(budget.js) + ' (' + kb(budget.gzip) + ' gzip) in plugins/sizeBudget.ts');
      }
      this.info(sizes + '; the budget is ' + kb(budget.js) + ' (' + kb(budget.gzip) + ' gzip)');
    },
  };
}
