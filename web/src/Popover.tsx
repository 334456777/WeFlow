// A floating layer anchored to a control (#224): it grows out of the control, stays on screen, and closes on a
// click outside or Esc. It lives in <body>, so a card's overflow never clips it.
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { createPortal } from 'react-dom';
import { usePresence } from './motion';

/** Distance from the anchor, and the least room kept to the window's edges. */
const GAP = 8;
const EDGE = 16;

interface Place { left: number; top: number; origin: string }

function placeOf(anchor: DOMRect, width: number, height: number): Place {
  const below = anchor.bottom + GAP + height <= window.innerHeight - EDGE || anchor.top - GAP - height < EDGE;
  const left = Math.max(EDGE, Math.min(anchor.left, window.innerWidth - EDGE - width));
  const top = below ? anchor.bottom + GAP : anchor.top - GAP - height;
  // grow out of the anchor's middle
  const origin = Math.round(anchor.left + anchor.width / 2 - left) + 'px ' + (below ? 'top' : 'bottom');
  return { left, top, origin };
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
      const box = anchor.current?.getBoundingClientRect();
      const layer = ref.current;
      if (box && layer) setPlace(placeOf(box, layer.offsetWidth, layer.offsetHeight));
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
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [open, anchor]);

  if (!shown) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={label} className={'popover' + (className ? ' ' + className : '')}
      data-state={shown.closing ? 'closing' : 'open'} inert={shown.closing}
      // until it is measured (before the first paint) it waits off screen, where it can already take the focus
      style={place ? { left: place.left, top: place.top, transformOrigin: place.origin } : { left: -9999, top: 0 }}
      onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); close.current('escape'); } }}>
      {children}
    </div>,
    document.body,
  );
}
