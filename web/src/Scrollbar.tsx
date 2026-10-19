// A scrollbar drawn by the page over its scroll area (`.scroll-area` hides the browser's): it takes no width, so
// content does not shift when it appears, and it looks like the rest of the page (a raised-sm thumb, #220).
// Render it right after the scroll area, in the same positioned parent; it places itself along its right edge.
// It also fades the area's top and bottom edges while there is more to scroll that way (#194): a layer in the
// surface's colour, as every surface of the page has that colour (#220), so it works on panels with shadows too,
// which a mask would clip.
import { useEffect, useRef } from 'react';
import type { RefObject } from 'react';

/** Distance from the scroll area's edges. */
const INSET = 4;
const MIN_THUMB = 24;
/** How long the bar stays after the last scroll. */
const LINGER_MS = 900;
/** Kept clear of the area's rounded corners by the fades; at the top and bottom they start at the edge, where a gap
 * would let a strip of content show. */
const FADE_INSET = 2;

export function Scrollbar({ target }: { target: RefObject<HTMLElement | null> }) {
  const track = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const fadeTop = useRef<HTMLDivElement>(null);
  const fadeBottom = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const area = target.current;
    const bar = track.current;
    const knob = thumb.current;
    if (!area || !bar || !knob) return;
    let travel = 0;
    let room = 0;
    let linger = 0;

    const fade = () => {
      const top = fadeTop.current;
      const bottom = fadeBottom.current;
      if (!top || !bottom) return;
      const overflow = room > 1 && area.clientHeight > 0;
      top.classList.toggle('shown', overflow && area.scrollTop > 1);
      bottom.classList.toggle('shown', overflow && area.scrollTop < room - 1);
      if (!overflow) return;
      const left = area.offsetLeft + area.clientLeft + FADE_INSET + 'px';
      const width = Math.max(0, area.clientWidth - 2 * FADE_INSET) + 'px';
      top.style.left = bottom.style.left = left;
      top.style.width = bottom.style.width = width;
      top.style.top = area.offsetTop + area.clientTop + 'px';
      bottom.style.top = area.offsetTop + area.clientTop + area.clientHeight - bottom.offsetHeight + 'px';
    };
    const place = () => {
      const { scrollHeight, clientHeight, scrollTop } = area;
      room = scrollHeight - clientHeight;
      const overflow = room > 1 && clientHeight > 0;
      bar.classList.toggle('scrollable', overflow);
      fade();
      if (!overflow) return;
      const height = clientHeight - 2 * INSET;
      bar.style.top = area.offsetTop + area.clientTop + INSET + 'px';
      bar.style.left = area.offsetLeft + area.clientLeft + area.clientWidth - bar.offsetWidth - INSET / 2 + 'px';
      bar.style.height = height + 'px';
      const size = Math.min(height, Math.max(MIN_THUMB, (height * clientHeight) / scrollHeight));
      travel = height - size;
      knob.style.height = size + 'px';
      knob.style.transform = 'translateY(' + (travel * Math.min(1, scrollTop / room)) + 'px)';
    };
    const show = () => {
      bar.classList.add('active');
      window.clearTimeout(linger);
      linger = window.setTimeout(() => bar.classList.remove('active'), LINGER_MS);
    };
    const onScroll = () => { place(); show(); };
    // Shown while the pointer is over the area or the thumb, which sits outside the area's element.
    const enter = () => { place(); bar.classList.add('hover'); };
    const leave = (event: PointerEvent) => {
      const to = event.relatedTarget as Node | null;
      if (to !== knob && !(to && area.contains(to))) bar.classList.remove('hover');
    };

    // The content grows and shrinks (cards open, rows arrive) without the area changing size: watch its children.
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(place);
    const watch = () => {
      if (resize) {
        resize.disconnect();
        resize.observe(area);
        for (const child of Array.from(area.children)) resize.observe(child);
      }
      place();
    };
    const children = new MutationObserver(watch);
    children.observe(area, { childList: true });
    watch();
    area.addEventListener('scroll', onScroll, { passive: true });
    area.addEventListener('pointerenter', enter);
    area.addEventListener('pointerleave', leave);

    // Dragging the thumb scrolls in proportion. Only the thumb takes the pointer: the strip beside it would cover
    // the right edge of rows and bubbles.
    let drag: { y: number; top: number } | null = null;
    const down = (event: PointerEvent) => {
      if (event.button !== 0) return;
      event.preventDefault();
      drag = { y: event.clientY, top: area.scrollTop };
      knob.setPointerCapture?.(event.pointerId);
      bar.classList.add('dragging');
    };
    const move = (event: PointerEvent) => {
      if (!drag || !travel) return;
      area.scrollTop = drag.top + ((event.clientY - drag.y) * room) / travel;
    };
    const up = () => { drag = null; bar.classList.remove('dragging'); show(); };
    knob.addEventListener('pointerdown', down);
    knob.addEventListener('pointerleave', leave);
    knob.addEventListener('pointermove', move);
    knob.addEventListener('pointerup', up);
    knob.addEventListener('pointercancel', up);

    return () => {
      resize?.disconnect();
      children.disconnect();
      window.clearTimeout(linger);
      area.removeEventListener('scroll', onScroll);
      area.removeEventListener('pointerenter', enter);
      area.removeEventListener('pointerleave', leave);
      knob.removeEventListener('pointerdown', down);
      knob.removeEventListener('pointerleave', leave);
      knob.removeEventListener('pointermove', move);
      knob.removeEventListener('pointerup', up);
      knob.removeEventListener('pointercancel', up);
    };
  }, [target]);

  return <>
    <div ref={fadeTop} className="scroll-fade top" aria-hidden="true" />
    <div ref={fadeBottom} className="scroll-fade bottom" aria-hidden="true" />
    <div ref={track} className="scrollbar" aria-hidden="true"><div ref={thumb} className="scrollbar-thumb" /></div>
  </>;
}
