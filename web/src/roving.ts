// 漫游焦点（roving focus，#195）：一组项目（导航、列表、芯片）在 Tab 键的顺序里只占一站。
// 方向键在组内移动（Home / End 到两端，列表里 PageUp / PageDown 一次移动十项），Tab 离开这一组。
// Tab 落到的是最后获得焦点的项目，没有时是选中的项目，再没有时是第一项。
//
// 组在它的元素上标出：`data-roving="vertical" | "horizontal" | "grid"`，
// `data-roving-items` 是组内项目的选择器（默认为按钮）。
// `data-roving-keys="off"` 保留 Tab 的一站，按键交给组自己处理（滚动时才生成行的虚拟列表）。
// 芯片组和分段控件不加标记也是组。
import { useEffect } from 'react';

const GROUPS = '[data-roving], .choice-chips[role="group"], .segmented[role="group"]';
const CHOSEN = '[aria-current="page"], [aria-current="true"], [aria-selected="true"], [aria-pressed="true"], .active, .selected';
const PAGE = 10;

type Orientation = 'vertical' | 'horizontal' | 'grid';

const orientationOf = (group: HTMLElement): Orientation =>
  (group.dataset.roving as Orientation | undefined) ?? (group.matches('.choice-chips, .segmented') ? 'horizontal' : 'vertical');

/** `group` 里能获得焦点的项目：只算它自己的，不算嵌在里面的组的项目。 */
export function itemsOf(group: HTMLElement): HTMLElement[] {
  const selector = group.dataset.rovingItems || 'button';
  return Array.from(group.querySelectorAll<HTMLElement>(selector)).filter(item =>
    item.closest(GROUPS) === group && !(item as HTMLButtonElement).disabled && item.getAttribute('aria-hidden') !== 'true');
}

/** 让应当接住 Tab 的项目成为这一组在 Tab 顺序里唯一的一站。 */
export function settle(group: HTMLElement, items = itemsOf(group)): void {
  if (!items.length) return;
  const focused = items.find(item => item === document.activeElement);
  const stop = focused ?? items.find(item => item.dataset.rovingStop === 'true') ?? items.find(item => item.matches(CHOSEN)) ?? items[0];
  // 只改有变化的：重新设一遍相同的 tabIndex 或标记仍算元素的一次改动，
  // 样式要重新计算，监视它的代码也会收到通知（#961）
  for (const item of items) {
    const index = item === stop ? 0 : -1;
    if (item.tabIndex !== index) item.tabIndex = index;
    if (item !== stop && item.dataset.rovingStop !== undefined) delete item.dataset.rovingStop;
  }
}

/** 页面的一次改动是否可能改变某一组在 Tab 顺序里的一站：组内的改动，或者新出现的组。 */
function touchesGroup(record: MutationRecord): boolean {
  const target = record.target instanceof Element ? record.target : record.target.parentElement;
  if (target?.closest(GROUPS)) return true;
  return Array.from(record.addedNodes).some(node => node instanceof Element && (node.matches(GROUPS) || node.querySelector(GROUPS) !== null));
}

/** 共 `count` 项时从第 `index` 项按下 `key` 后移到的位置；组不处理这个键时为 null。 */
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

/** 网格第一行的项目数。 */
const columnsOf = (items: HTMLElement[]) => {
  const top = items[0]?.getBoundingClientRect().top;
  const n = items.findIndex(item => item.getBoundingClientRect().top > (top ?? 0) + 1);
  return n > 0 ? n : items.length || 1;
};

const typing = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || target.matches('input:not([type="checkbox"]):not([type="radio"]), textarea, select'));

/** 给页面上的每一组装上漫游焦点，之后出现的组也会装上。 */
export function useRovingFocus(): void {
  useEffect(() => {
    let queued = 0;
    const settleAll = () => {
      queued = 0;
      document.querySelectorAll<HTMLElement>(GROUPS).forEach(group => settle(group));
    };
    const later = () => { if (!queued) queued = requestAnimationFrame(settleAll); };
    settleAll();
    // 所有组以外的改动（出现滚动条、面板折叠）不影响各组
    const watch = new MutationObserver(records => { if (records.some(touchesGroup)) later(); });
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
    // 点击把焦点移到哪一项，Tab 回到这一组时就落在哪一项
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

/** 按 `/` 把焦点放到第一个显示着的搜索框；有打开的对话框或正在别处输入时不处理。 */
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
