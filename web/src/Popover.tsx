// A floating layer anchored to a control (#224): it grows out of the control, stays on screen, and closes on a
// click outside or Esc. It lives in <body>, so a card's overflow never clips it.
//
// It opens below the control, or above where only that has room. Where neither has, it takes the larger side and
// scrolls inside; on a screen narrower than it plus the margins, the margins shrink, then it scrolls across too
// (#239). While its control is scrolled out of sight (of the window, or of a list it sits in) it hides, and shows
// again when the control is back.
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, RefObject } from 'react';
import { createPortal } from 'react-dom';
import { usePresence } from './motion';

/** Distance from the anchor, and the least room kept to the window's edges. */
const GAP = 8;
const EDGE = 16;

interface Place { left: number; top: number; origin: string; maxWidth?: number; maxHeight?: number; hidden: boolean }

/** The part of the anchor not clipped by the window or by an ancestor that scrolls or clips. */
export function visibleBox(anchor: HTMLElement): { top: number; bottom: number } {
  const box = anchor.getBoundingClientRect();
  let top = Math.max(box.top, 0);
  let bottom = Math.min(box.bottom, window.innerHeight);
  for (let p = anchor.parentElement; p && p !== document.body; p = p.parentElement) {
    if (getComputedStyle(p).overflowY === 'visible') continue;
    const clip = p.getBoundingClientRect();
    top = Math.max(top, clip.top);
    bottom = Math.min(bottom, clip.bottom);
  }
  return { top, bottom };
}

/** Where a layer of this natural size goes next to the anchor. */
export function placeOf(anchor: DOMRect, width: number, height: number, hidden = false): Place {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const edge = Math.max(0, Math.min(EDGE, (vw - width) / 2));
  const maxWidth = width > vw - 2 * edge ? vw - 2 * edge : undefined;
  const left = Math.max(edge, Math.min(anchor.left, vw - edge - Math.min(width, vw)));
  const roomBelow = vh - EDGE - anchor.bottom - GAP;
  const roomAbove = anchor.top - GAP - EDGE;
  const below = height <= roomBelow || (height > roomAbove && roomBelow >= roomAbove);
  const room = Math.max(0, below ? roomBelow : roomAbove);
  const maxHeight = height > room ? room : undefined;
  const top = below ? anchor.bottom + GAP : anchor.top - GAP - Math.min(height, room);
  // grow out of the anchor's middle
  const origin = Math.round(anchor.left + anchor.width / 2 - left) + 'px ' + (below ? 'top' : 'bottom');
  return { left, top, origin, maxWidth, maxHeight, hidden };
}

function styleOf(place: Place | null): CSSProperties {
  // until it is measured (before the first paint) it waits off screen, where it can already take the focus
  if (!place) return { left: -9999, top: 0 };
  const scrolls = place.maxHeight !== undefined || place.maxWidth !== undefined;
  return {
    left: place.left, top: place.top, transformOrigin: place.origin, maxWidth: place.maxWidth, maxHeight: place.maxHeight,
    overflow: scrolls ? 'auto' : undefined, visibility: place.hidden ? 'hidden' : undefined,
  };
}

export function Popover({ anchor, open, onClose, label, className, children }: {
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  /** Esc returns the focus to the control; a click outside leaves it where the click put it. */
  onClose: (reason: 'escape' | 'outside') => void;
  label: string;
  className?: string;
  children: ReactNode;
}) {
  const shown = usePresence(open || null);
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useLayoutEffect(() => {
    if (!shown) { setPlace(null); return; }
    const update = () => {
      const control = anchor.current;
      const layer = ref.current;
      if (!control || !layer) return;
      const seen = visibleBox(control);
      // the natural size, scrolled or not: its content and padding (it has no border)
      setPlace(placeOf(control.getBoundingClientRect(), layer.scrollWidth, layer.scrollHeight, seen.bottom < seen.top));
    };
    update();
    // the console and the page scroll under it: follow the anchor
    window.addEventListener('scroll', update, true);
    window.addEventListener('resize', update);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    if (ref.current) observer?.observe(ref.current);
    return () => {
      window.removeEventListener('scroll', update, true);
      window.removeEventListener('resize', update);
      observer?.disconnect();
    };
  }, [Boolean(shown), anchor]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!ref.current?.contains(target) && !anchor.current?.contains(target)) close.current('outside');
    };
    // Esc with the focus nowhere (on <body>) closes it too; with the focus in it, its own handler below does
    const escape = (event: KeyboardEvent) => {
      const at = document.activeElement;
      if (event.key === 'Escape' && (!at || at === document.body)) close.current('escape');
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape);
    };
  }, [open, anchor]);

  if (!shown) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={label} className={'popover' + (className ? ' ' + className : '')}
      data-state={shown.closing ? 'closing' : 'open'} inert={shown.closing}
      style={styleOf(place)}
      onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); close.current('escape'); } }}>
      {children}
    </div>,
    document.body,
  );
}
