// 一列卡片，点标题栏展开或收起，可以拖动调整顺序。顺序和展开状态保存在这个浏览器里（localStorage）。
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { CaretDown } from '@phosphor-icons/react';
import { timing } from './motion';
import { t } from './i18n';

/** 卡片列里的一张卡片：标题栏和内容。 */
export interface Card {
  id: string;
  title: string;
  /** 卡片收起时显示在标题栏右边。 */
  summary?: ReactNode;
  /** 卡片收起后内容仍然保留（只是折叠起来）；只在展开时读取的内容写成函数，接收 `open`。 */
  content: ReactNode | ((open: boolean) => ReactNode);
}

/** 这个浏览器保存的卡片列布局：卡片的顺序和收起的卡片，都按 ID 记录。 */
export interface CardLayout {
  order: string[];
  closed: string[];
}

/** 在标题栏上按下后，指针移动超过这么多像素才算拖动。 */
const DEAD_ZONE = 4;

/** 读取保存的布局，只保留认识的卡片，新的卡片加在最后；没有保存的布局时用默认的顺序和 `closed`。 */
export function loadLayout(key: string, ids: string[], closed: string[]): CardLayout {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) || 'null');
    if (saved && typeof saved === 'object' && 'order' in saved && 'closed' in saved && Array.isArray(saved.order) && Array.isArray(saved.closed)) {
      const order = saved.order.filter((id): id is string => ids.includes(id));
      return { order: [...order, ...ids.filter(id => !order.includes(id))], closed: saved.closed.filter((id): id is string => ids.includes(id)) };
    }
  } catch { /* 没有存储，或者保存的内容坏了：从默认值开始 */ }
  return { order: ids, closed };
}

function cardElements(container: HTMLElement | null): HTMLElement[] {
  return container ? Array.from(container.querySelectorAll<HTMLElement>(':scope > [data-card]')) : [];
}

function tops(container: HTMLElement | null): Map<string, number> {
  return new Map(cardElements(container).map(element => [element.dataset.card!, element.getBoundingClientRect().top]));
}

interface Drag {
  id: string;
  element: HTMLElement;
  startY: number;
  pointerY: number;
  /** 卡片上边缘到指针的距离。 */
  grab: number;
  /** 卡片当前相对于它在列中位置的偏移。 */
  offset: number;
  active: boolean;
}

/** 按保存在 `storageKey` 下的顺序排列的卡片；`defaultClosed` 里的卡片在展开之前是收起的。 */
export function CardStack({ cards, storageKey, defaultClosed, label, hidden = [] }: {
  cards: Card[]; storageKey: string; defaultClosed: string[]; label: string;
  /** 暂时不显示的卡片；它们在保存的顺序里保留位置，再显示时回到原处。 */
  hidden?: readonly string[];
}) {
  const [layout, setLayout] = useState(() => loadLayout(storageKey, cards.map(card => card.id), defaultClosed));
  const hiddenNow = useRef(hidden);
  hiddenNow.current = hidden;
  const [dragging, setDragging] = useState<string | null>(null);
  const container = useRef<HTMLDivElement>(null);
  const current = useRef(layout);
  const before = useRef<{ tops: Map<string, number>; skip?: string; focus?: string } | null>(null);
  const drag = useRef<Drag | null>(null);
  const swallowClick = useRef(false);
  const stopDrag = useRef<(() => void) | null>(null);

  useEffect(() => () => stopDrag.current?.(), []);

  const save = (next: CardLayout) => {
    current.current = next;
    setLayout(next);
    try { localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* 写不进去时，只是刷新后顺序会丢失 */ }
  };

  /** 把一张卡片移到显示的卡片中的第 `index` 位；隐藏的卡片在整个顺序里留在原来的位置。 */
  const move = (id: string, index: number, options: { skip?: string; focus?: string } = {}) => {
    const all = current.current.order;
    const shown = all.filter(other => !hiddenNow.current.includes(other));
    const rest = shown.filter(other => other !== id);
    if (index < 0 || index > rest.length || shown.indexOf(id) === index) return;
    rest.splice(index, 0, id);
    let next = 0;
    const order = all.map(other => (hiddenNow.current.includes(other) ? other : rest[next++]));
    before.current = { tops: tops(container.current), ...options };
    save({ ...current.current, order });
  };

  const toggle = (id: string) => {
    const { closed } = current.current;
    save({ ...current.current, closed: closed.includes(id) ? closed.filter(other => other !== id) : [...closed, id] });
  };

  // 调整顺序之后：其他卡片从原来的位置滑过去（FLIP），拖动的卡片保持在指针下面。
  useLayoutEffect(() => {
    const previous = before.current;
    before.current = null;
    if (!previous) return;
    const held = drag.current;
    if (held?.active) {
      const layoutTop = held.element.getBoundingClientRect().top - held.offset;
      held.offset = held.pointerY - held.grab - layoutTop;
      held.element.style.transform = 'translateY(' + String(held.offset) + 'px)';
    }
    const settle = timing('--duration-panel', '--ease-snap');
    for (const element of cardElements(container.current)) {
      const id = element.dataset.card!;
      const delta = (previous.tops.get(id) ?? 0) - element.getBoundingClientRect().top;
      if (id !== previous.skip && settle.duration && Math.abs(delta) >= 1) {
        element.animate?.([{ transform: 'translateY(' + String(delta) + 'px)' }, { transform: 'translateY(0)' }], settle);
      }
      if (id === previous.focus) element.querySelector<HTMLElement>('.card-header')?.focus();
    }
  }, [layout.order]);

  const endDrag = () => {
    window.removeEventListener('pointermove', followPointer);
    window.removeEventListener('pointerup', endDrag);
    window.removeEventListener('pointercancel', endDrag);
    stopDrag.current = null;
    const held = drag.current;
    drag.current = null;
    if (!held?.active) return;
    setDragging(null);
    // 按 `--ease-snap` 的缓动曲线落到位。
    held.element.style.transform = '';
    const settle = timing('--duration-panel', '--ease-snap');
    if (settle.duration) held.element.animate?.([{ transform: 'translateY(' + String(held.offset) + 'px)' }, { transform: 'translateY(0)' }], settle);
  };

  function followPointer(event: PointerEvent) {
    const held = drag.current;
    if (!held) return;
    held.pointerY = event.clientY;
    if (!held.active) {
      if (Math.abs(event.clientY - held.startY) < DEAD_ZONE) return;
      held.active = true;
      swallowClick.current = true;
      setDragging(held.id);
    }
    const rect = held.element.getBoundingClientRect();
    const layoutTop = rect.top - held.offset;
    held.offset = event.clientY - held.grab - layoutTop;
    held.element.style.transform = 'translateY(' + String(held.offset) + 'px)';
    // 卡片的中线越过了哪些卡片的中线，就排到它们后面。
    const middle = event.clientY - held.grab + rect.height / 2;
    const index = cardElements(container.current)
      .filter(element => element !== held.element)
      .filter(element => { const other = element.getBoundingClientRect(); return other.top + other.height / 2 < middle; }).length;
    move(held.id, index, { skip: held.id });
  }

  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>, id: string) => {
    swallowClick.current = false;
    // 触摸仍用来滚动页面；卡片用鼠标、触控笔或键盘移动。
    if (event.button !== 0 || event.pointerType === 'touch') return;
    const element = event.currentTarget.closest<HTMLElement>('[data-card]');
    if (!element) return;
    drag.current = { id, element, startY: event.clientY, pointerY: event.clientY, grab: event.clientY - element.getBoundingClientRect().top, offset: 0, active: false };
    window.addEventListener('pointermove', followPointer);
    window.addEventListener('pointerup', endDrag);
    window.addEventListener('pointercancel', endDrag);
    stopDrag.current = endDrag;
  };

  const moveWithKeys = (event: ReactKeyboardEvent<HTMLButtonElement>, id: string) => {
    if (!event.altKey || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
    event.preventDefault();
    const shown = current.current.order.filter(other => !hiddenNow.current.includes(other));
    move(id, shown.indexOf(id) + (event.key === 'ArrowUp' ? -1 : 1), { focus: id });
  };

  const byId = new Map(cards.map(card => [card.id, card]));
  return <div className="card-stack" ref={container} role="list" aria-label={label} data-roving="vertical" data-roving-items=".card-header">
    {layout.order.map(id => {
      const card = byId.get(id);
      if (!card || hidden.includes(id)) return null;
      const open = !layout.closed.includes(id);
      return <section key={id} role="listitem" data-card={id} className={'card' + (open ? '' : ' collapsed') + (dragging === id ? ' dragging' : '')} aria-labelledby={'card-' + id}>
        <h2 className="card-heading">
          <button type="button" id={'card-' + id} className="card-header" aria-expanded={open} aria-controls={'card-body-' + id}
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown" title={t('点击展开或收起；拖动，或按 Alt+↑ / Alt+↓ 调整顺序')}
            onPointerDown={event => startDrag(event, id)} onKeyDown={event => moveWithKeys(event, id)}
            onClick={() => { if (swallowClick.current) swallowClick.current = false; else toggle(id); }}>
            <span className="card-title">{card.title}</span><CaretDown size={14} className="card-caret" aria-hidden="true" />
            {card.summary !== undefined && <span className="card-summary" aria-hidden={open}>{card.summary}</span>}
          </button>
        </h2>
        <div className="card-body" id={'card-body-' + id} inert={!open}><div className="card-inner"><div className="card-content">{typeof card.content === 'function' ? card.content(open) : card.content}</div></div></div>
      </section>;
    })}
  </div>;
}
