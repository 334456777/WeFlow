// The colour theme (#252): light, dark, or the system's, which is the default. The choice is kept in this
// browser's localStorage; the theme in force is `data-theme` on <html>, which tokens.css reads. index.html sets it
// before the first paint with the same rules, so a page does not flash the other theme while it loads.
import { useEffect, useState } from 'react';

export type ThemeChoice = 'system' | 'light' | 'dark';

export const THEME_KEY = 'weflow.theme';
const DARK = '(prefers-color-scheme: dark)';

export function savedTheme(): ThemeChoice {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return value === 'light' || value === 'dark' ? value : 'system';
  } catch {
    return 'system';
  }
}

/** The theme a choice comes to now: the system's light or dark mode for `system`. */
export function themeOf(choice: ThemeChoice): 'light' | 'dark' {
  if (choice !== 'system') return choice;
  return window.matchMedia?.(DARK).matches ? 'dark' : 'light';
}

/** The chosen theme, applied to the page and kept; `system` follows the system as it changes. */
export function useTheme(): [ThemeChoice, (choice: ThemeChoice) => void] {
  const [choice, setChoice] = useState<ThemeChoice>(savedTheme);
  useEffect(() => {
    const apply = () => { document.documentElement.dataset.theme = themeOf(choice); };
    apply();
    if (choice !== 'system') return;
    const query = window.matchMedia?.(DARK);
    query?.addEventListener('change', apply);
    return () => query?.removeEventListener('change', apply);
  }, [choice]);
  const choose = (next: ThemeChoice) => {
    setChoice(next);
    try {
      if (next === 'system') localStorage.removeItem(THEME_KEY);
      else localStorage.setItem(THEME_KEY, next);
    } catch {
      // not kept: the choice holds for this page
    }
  };
  return [choice, choose];
}
