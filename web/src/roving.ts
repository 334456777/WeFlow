// Roving focus (#195): a group of items — the navigation, a list, chips — is one stop for Tab. The arrow keys move
// among its items (Home / End to either end, PageUp / PageDown ten at a time in a list), and Tab leaves the group.
// The item Tab lands on is the one last focused, else the chosen one, else the first.
//
// A group is marked on its element: `data-roving="vertical" | "horizontal" | "grid"`, and `data-roving-items` the
// selector of its items (buttons by default). `data-roving-keys="off"` keeps the Tab stop but leaves the keys to the
// group (a virtual list that scrolls rows into being). Groups of chips and segmented buttons are groups without marks.
import { useEffect } from 'react';

const GROUPS = '[data-roving], .choice-chips[role="group"], .segmented[role="group"]';
const CHOSEN = '[aria-current="page"], [aria-current="true"], [aria-selected="true"], [aria-pressed="true"], .active, .selected';
const PAGE = 10;

type Orientation = 'vertical' | 'horizontal' | 'grid';

const orientationOf = (group: HTMLElement): Orientation =>
  (group.dataset.roving as Orientation | undefined) ?? (group.matches('.choice-chips, .segmented') ? 'horizontal' : 'vertical');

/** The items of `group` that can take the focus: its own, not those of a group inside it. */
export function itemsOf(group: HTMLElement): HTMLElement[] {
  const selector = group.dataset.rovingItems || 'button';
  return Array.from(group.querySelectorAll<HTMLElement>(selector)).filter(item =>
    item.closest(GROUPS) === group && !(item as HTMLButtonElement).disabled && item.getAttribute('aria-hidden') !== 'true');
}

/** Makes the item that should take Tab the group's only stop. */
export function settle(group: HTMLElement, items = itemsOf(group)): void {
  if (!items.length) return;
  const focused = items.find(item => item === document.activeElement);
  const stop = focused ?? items.find(item => item.dataset.rovingStop === 'true') ?? items.find(item => item.matches(CHOSEN)) ?? items[0];
  for (const item of items) {
    item.tabIndex = item === stop ? 0 : -1;
    if (item !== stop) delete item.dataset.rovingStop;
  }
}

/** The index the key moves to from `index` among `count`, or null for a key the group does not take. */
export function step(key: string, index: number, count: number, orientation: Orientation, columns = 1): number | null {
  const last = count - 1;
  const clamp = (i: number) => Math.max(0, Math.min(last, i));
  const across = orientation === 'grid' ? columns : 1;
  const map: Record<string, number | undefined> = {
    Home: 0, End: last,
    ArrowDown: orientation === 'horizontal' ? undefined : clamp(index + across),
    ArrowUp: orientation === 'horizontal' ? undefined : clamp(index - across),
    ArrowRight: orientation === 'vertical' ? undefined : clamp(index + 1),
    ArrowLeft: orientation === 'vertical' ? undefined : clamp(index - 1),
    PageDown: orientation === 'vertical' ? clamp(index + PAGE) : undefined,
    PageUp: orientation === 'vertical' ? clamp(index - PAGE) : undefined,
  };
  const to = map[key];
  return to === undefined ? null : to;
}

/** How many items of a grid sit on its first row. */
const columnsOf = (items: HTMLElement[]) => {
  const top = items[0]?.getBoundingClientRect().top;
  const n = items.findIndex(item => item.getBoundingClientRect().top > (top ?? 0) + 1);
  return n > 0 ? n : items.length || 1;
};

const typing = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || target.matches('input:not([type="checkbox"]):not([type="radio"]), textarea, select'));

/** Installs roving focus on every group of the page, now and as they come. */
export function useRovingFocus(): void {
  useEffect(() => {
    let queued = 0;
    const settleAll = () => {
      queued = 0;
      document.querySelectorAll<HTMLElement>(GROUPS).forEach(group => settle(group));
    };
    const later = () => { if (!queued) queued = requestAnimationFrame(settleAll); };
    settleAll();
    const watch = new MutationObserver(later);
    watch.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['aria-pressed', 'aria-selected', 'aria-current', 'class', 'disabled'] });

    const keydown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || typing(event.target)) return;
      const target = event.target as HTMLElement;
      const group = target.closest<HTMLElement>(GROUPS);
      if (!group || group.dataset.rovingKeys === 'off') return;
      const items = itemsOf(group);
      const index = items.indexOf(target.closest<HTMLElement>(group.dataset.rovingItems || 'button') ?? target);
      if (index < 0) return;
      const orientation = orientationOf(group);
      const to = step(event.key, index, items.length, orientation, orientation === 'grid' ? columnsOf(items) : 1);
      if (to === null) return;
      event.preventDefault();
      const next = items[to];
      next.dataset.rovingStop = 'true';
      settle(group, items);
      next.tabIndex = 0;
      next.focus({ preventScroll: true });
      next.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    };
    // where the focus goes by a click, Tab comes back to
    const focusin = (event: FocusEvent) => {
      const target = event.target as HTMLElement;
      const group = target.closest?.<HTMLElement>(GROUPS);
      if (!group) return;
      const items = itemsOf(group);
      if (!items.includes(target)) return;
      target.dataset.rovingStop = 'true';
      settle(group, items);
    };
    document.addEventListener('keydown', keydown);
    document.addEventListener('focusin', focusin);
    return () => {
      cancelAnimationFrame(queued);
      watch.disconnect();
      document.removeEventListener('keydown', keydown);
      document.removeEventListener('focusin', focusin);
    };
  }, []);
}

/** `/` puts the focus in the first search field showing, as in many sites; not while typing elsewhere. */
export function useSearchKey(): void {
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || typing(event.target)) return;
      if (document.querySelector('dialog[open]')) return;
      const field = Array.from(document.querySelectorAll<HTMLInputElement>('main input[type="search"]')).find(input => input.offsetParent !== null);
      if (!field) return;
      event.preventDefault();
      field.focus();
      field.select();
    };
    document.addEventListener('keydown', keydown);
    return () => document.removeEventListener('keydown', keydown);
  }, []);
}
