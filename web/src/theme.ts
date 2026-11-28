// 配色主题（#252）：浅色、深色或跟随系统（默认）。选择保存在这个浏览器的 localStorage 里；生效的主题是 <html> 上的
// `data-theme`，皮肤的颜色跟着它变。index.html 在第一次绘制之前按同样的规则设好它，页面加载时不会先闪现另一种主题。
import { useEffect, useState } from 'react';

/** 选择的主题：浅色、深色或跟随系统。 */
export type ThemeChoice = 'system' | 'light' | 'dark';

/** 选择的主题保存在 `localStorage` 里所用的键。 */
export const THEME_KEY = 'weflow.theme';
const DARK = '(prefers-color-scheme: dark)';

/** 这个浏览器里选择的主题；没有选过或无法读取存储时为跟随系统。 */
export function savedTheme(): ThemeChoice {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return value === 'light' || value === 'dark' ? value : 'system';
  } catch {
    return 'system';
  }
}

/** 一个选择此刻对应的主题：`system` 时为系统当前的浅色或深色模式。 */
export function themeOf(choice: ThemeChoice): 'light' | 'dark' {
  if (choice !== 'system') return choice;
  return window.matchMedia?.(DARK).matches ? 'dark' : 'light';
}

/** 选择的主题和改选的函数：主题应用到页面上并保存下来，`system` 随系统的变化切换。 */
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
      // 无法保存：这个选择只在当前页面里有效
    }
  };
  return [choice, choose];
}
