// A column of cards that open and close from their header and can be dragged into a new order, like the side
// panel of Claude Code. Order and open state stay in this browser (localStorage).
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { CaretDown } from '@phosphor-icons/react';
import { durationMs } from './motion';

export interface Card {
  id: string;
  title: string;
  /** Shown on the right of the header while the card is closed. */
  summary?: ReactNode;
  content: ReactNode;
}

export interface CardLayout {
  order: string[];
  closed: string[];
}

/** Pointer travel before a press on a header becomes a drag. */
const DEAD_ZONE = 4;

/** Reads a saved layout, keeping only known cards and adding new ones at the end. */
export function loadLayout(key: string, ids: string[], closed: string[]): CardLayout {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) || 'null');
    if (saved && typeof saved === 'object' && 'order' in saved && 'closed' in saved && Array.isArray(saved.order) && Array.isArray(saved.closed)) {
      const order = saved.order.filter((id): id is string => ids.includes(id));
      return { order: [...order, ...ids.filter(id => !order.includes(id))], closed: saved.closed.filter((id): id is string => ids.includes(id)) };
    }
  } catch { /* no storage, or a broken entry: start from the defaults */ }
  return { order: ids, closed };
}

function cardElements(container: HTMLElement | null): HTMLElement[] {
  return container ? Array.from(container.querySelectorAll<HTMLElement>(':scope > [data-card]')) : [];
}

function tops(container: HTMLElement | null): Map<string, number> {
  return new Map(cardElements(container).map(element => [element.dataset.card!, element.getBoundingClientRect().top]));
}

function snapEasing(): string {
  return getComputedStyle(document.documentElement).getPropertyValue('--ease-snap').trim() || 'ease';
}

interface Drag {
  id: string;
  element: HTMLElement;
  startY: number;
  pointerY: number;
  /** Distance from the card's top edge to the pointer. */
  grab: number;
  /** Current offset of the card from its place in the column. */
  offset: number;
  active: boolean;
}

export function CardStack({ cards, storageKey, defaultClosed, label }: {
  cards: Card[]; storageKey: string; defaultClosed: string[]; label: string;
}) {
  const [layout, setLayout] = useState(() => loadLayout(storageKey, cards.map(card => card.id), defaultClosed));
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
    try { localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* the order is only lost on reload */ }
  };

  const move = (id: string, index: number, options: { skip?: string; focus?: string } = {}) => {
    const order = current.current.order.filter(other => other !== id);
    if (index < 0 || index > order.length || current.current.order.indexOf(id) === index) return;
    order.splice(index, 0, id);
    before.current = { tops: tops(container.current), ...options };
    save({ ...current.current, order });
  };

  const toggle = (id: string) => {
    const { closed } = current.current;
    save({ ...current.current, closed: closed.includes(id) ? closed.filter(other => other !== id) : [...closed, id] });
  };

  // After a reorder: slide the other cards from where they were (FLIP), keep the dragged card under the pointer.
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
    const duration = durationMs('--duration-panel');
    for (const element of cardElements(container.current)) {
      const id = element.dataset.card!;
      const delta = (previous.tops.get(id) ?? 0) - element.getBoundingClientRect().top;
      if (id !== previous.skip && duration && Math.abs(delta) >= 1) {
        element.animate?.([{ transform: 'translateY(' + String(delta) + 'px)' }, { transform: 'translateY(0)' }], { duration, easing: snapEasing() });
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
    // Settle into place on the snap curve.
    held.element.style.transform = '';
    const duration = durationMs('--duration-panel');
    if (duration) held.element.animate?.([{ transform: 'translateY(' + String(held.offset) + 'px)' }, { transform: 'translateY(0)' }], { duration, easing: snapEasing() });
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
    // The card goes after every other card whose middle its own middle has passed.
    const middle = event.clientY - held.grab + rect.height / 2;
    const index = cardElements(container.current)
      .filter(element => element !== held.element)
      .filter(element => { const other = element.getBoundingClientRect(); return other.top + other.height / 2 < middle; }).length;
    move(held.id, index, { skip: held.id });
  }

  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>, id: string) => {
    swallowClick.current = false;
    // Touch keeps scrolling the page; cards move with a mouse, a pen or the keyboard.
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
    move(id, current.current.order.indexOf(id) + (event.key === 'ArrowUp' ? -1 : 1), { focus: id });
  };

  const byId = new Map(cards.map(card => [card.id, card]));
  return <div className="card-stack" ref={container} role="list" aria-label={label}>
    {layout.order.map(id => {
      const card = byId.get(id);
      if (!card) return null;
      const open = !layout.closed.includes(id);
      return <section key={id} role="listitem" data-card={id} className={'card' + (open ? '' : ' collapsed') + (dragging === id ? ' dragging' : '')} aria-labelledby={'card-' + id}>
        <h2 className="card-heading">
          <button type="button" id={'card-' + id} className="card-header" aria-expanded={open} aria-controls={'card-body-' + id}
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown" title="点击展开或收起；拖动，或按 Alt+↑ / Alt+↓ 调整顺序"
            onPointerDown={event => startDrag(event, id)} onKeyDown={event => moveWithKeys(event, id)}
            onClick={() => { if (swallowClick.current) swallowClick.current = false; else toggle(id); }}>
            <span className="card-title">{card.title}</span><CaretDown size={14} className="card-caret" aria-hidden="true" />
            {card.summary !== undefined && <span className="card-summary" aria-hidden={open}>{card.summary}</span>}
          </button>
        </h2>
        <div className="card-body" id={'card-body-' + id} inert={!open}><div className="card-inner"><div className="card-content">{card.content}</div></div></div>
      </section>;
    })}
  </div>;
}
