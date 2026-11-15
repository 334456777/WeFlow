// The page is written with React's components and hooks, and the Vite build renders it with Preact (#783).
// react-dom is about 66 kB gzip of the first screen. Preact's compatibility layer covers the hooks, portals and
// flushSync the page uses, so the build stays inside the size budget with room to spare. Tests keep React: Preact's
// scheduler and Testing Library's `act` do not flush the same turns, and bending the tests to match changes what
// they assert.

/** Bare specifiers the page imports, and the Preact module each one is. */
export const REACT_TO_PREACT: Record<string, string> = {
  'react': 'preact/compat',
  'react/jsx-runtime': 'preact/jsx-runtime',
  'react/jsx-dev-runtime': 'preact/jsx-dev-runtime',
  'react-dom': 'preact/compat',
  'react-dom/client': 'preact/compat/client',
};

/** Vite aliases. Exact, so `react` does not swallow `react/jsx-runtime` before that entry is reached. */
export function viteReactAlias(): { find: RegExp; replacement: string }[] {
  return Object.entries(REACT_TO_PREACT).map(([from, to]) => ({
    find: new RegExp('^' + from.replace('/', '\\/') + '$'),
    replacement: to,
  }));
}
