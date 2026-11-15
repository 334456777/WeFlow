import { join } from 'node:path';
import { describe, expect, it } from '#test';

const src = join(import.meta.dir, '..', 'src');

describe('the imports from React (#783)', () => {
  it('names what the page takes from react and react-dom', async () => {
    // `React.StrictMode` reads preact/compat's default export, which holds the whole layer; the build then keeps
    // lazy, Suspense, hydrate and the rest of it
    const whole = /^import\s+(?!type\b)(?:\w+|\*\s*as\s+\w+)\s*(?:,\s*\{[^}]*\})?\s*from\s*['"]react(?:-dom)?(?:\/[\w-]+)?['"]/m;
    const offending: string[] = [];
    for await (const path of new Bun.Glob('**/*.{ts,tsx}').scan({ cwd: src })) {
      if (/\.test\.tsx?$/.test(path)) continue;
      if (whole.test(await Bun.file(join(src, path)).text())) offending.push(path);
    }
    expect(offending).toEqual([]);
    expect(whole.test("import React from 'react';")).toBe(true);
    expect(whole.test("import * as ReactDOM from 'react-dom/client';")).toBe(true);
    expect(whole.test("import { StrictMode } from 'react';")).toBe(false);
    expect(whole.test("import type { ReactNode } from 'react';")).toBe(false);
  });
});
