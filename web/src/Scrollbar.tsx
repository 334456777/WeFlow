// A scrollbar drawn by the page over its scroll area (`.scroll-area` hides the browser's): it takes no width, so
// content does not shift when it appears, and it looks like the rest of the page (the skin's scrollbar role, #828).
// Render it right after the scroll area, in the same positioned parent; it places itself along its right edge.
// It also fades the area's top and bottom edges while there is more to scroll that way (#194, #831): a frame over the
// area, of its size, its corners and its clip whatever they are, holds a band at each edge, which app.css paints with
// what lies under the area and shows while there is more beyond that edge (`shown`).
import { useEffect, useRef } from 'react';
import type { RefObject } from 'react';

/** Distance from the scroll area's edges. */
const INSET = 4;
const MIN_THUMB = 24;
/** How long the bar stays after the last scroll. */
const LINGER_MS = 900;
/** Sent to the scroll area when a person starts dragging the thumb, which is not inside it: the area's own pointer
 * events do not see that the person is scrolling (a chat window lets go of its bottom on it). */
export const SCROLL_INTENT = 'weflow:scroll-intent';

/** Whether the pointer is over `element`; false where `:hover` cannot be asked. */
function hovered(element: Element) {
  try { return element.matches(':hover'); } catch { return false; }
}

export function Scrollbar({ target }: { target: RefObject<HTMLElement | null> }) {
  const track = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const fades = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const area = target.current;
    const bar = track.current;
    const knob = thumb.current;
    if (!area || !bar || !knob) return;
    let travel = 0;
    let room = 0;
    let linger = 0;

    // It runs at every frame of a scroll, and of a panel that grows or shrinks (the sidebar folding): everything is
    // read first and only then written, so that the page is laid out once a frame and not again for each read (#961),
    // and a style is set only when it changes.
    const set = (element: HTMLElement, property: 'left' | 'top' | 'width' | 'height' | 'borderRadius' | 'clipPath' | 'transform', value: string) => {
      if (element.style[property] !== value) element.style[property] = value;
    };
    const place = () => {
      const { scrollHeight, clientHeight, scrollTop } = area;
      room = scrollHeight - clientHeight;
      const overflow = room > 1 && clientHeight > 0;
      const left = area.offsetLeft + area.clientLeft;
      const top = area.offsetTop + area.clientTop;
      const width = area.clientWidth;
      // the bar's own width, from its style: it takes none on the page while there is nothing to scroll
      const barWidth = overflow ? Number.parseFloat(getComputedStyle(bar).width) || 0 : 0;
      // the area's shape, whatever the skin makes it: its corners and its clip
      const shape = overflow && fades.current ? getComputedStyle(area) : null;
      const radius = shape?.borderRadius ?? '';
      const clip = shape?.clipPath ?? '';

      bar.classList.toggle('scrollable', overflow);
      const frame = fades.current;
      if (frame) {
        const [above, below] = frame.firstElementChild!.children;
        frame.classList.toggle('scrollable', overflow);
        above.classList.toggle('shown', overflow && scrollTop > 1);
        below.classList.toggle('shown', overflow && scrollTop < room - 1);
        if (overflow) {
          set(frame, 'left', left + 'px');
          set(frame, 'top', top + 'px');
          set(frame, 'width', width + 'px');
          set(frame, 'height', clientHeight + 'px');
          set(frame, 'borderRadius', radius);
          set(frame, 'clipPath', clip);
        }
      }
      if (!overflow) return;
      const height = clientHeight - 2 * INSET;
      set(bar, 'top', top + INSET + 'px');
      set(bar, 'left', left + width - barWidth + 'px');
      set(bar, 'height', height + 'px');
      const size = Math.min(height, Math.max(MIN_THUMB, (height * clientHeight) / scrollHeight));
      travel = height - size;
      set(knob, 'height', size + 'px');
      set(knob, 'transform', 'translateY(' + (travel * Math.min(1, scrollTop / room)) + 'px)');
    };
    const show = () => {
      // added only when it is not there: adding it again would still be a change of the class to whoever watches
      if (!bar.classList.contains('active')) bar.classList.add('active');
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
    // a pointer already over a new area (another page in the same panel, #836) sends it no `pointerenter`
    if (hovered(area)) bar.classList.add('hover');

    // Dragging the thumb scrolls in proportion. Only the thumb takes the pointer: the strip beside it would cover
    // the right edge of rows and bubbles.
    let drag: { y: number; top: number } | null = null;
    const down = (event: PointerEvent) => {
      if (event.button !== 0) return;
      event.preventDefault();
      drag = { y: event.clientY, top: area.scrollTop };
      area.dispatchEvent(new Event(SCROLL_INTENT));
      knob.setPointerCapture?.(event.pointerId);
      bar.classList.add('dragging');
    };
    const move = (event: PointerEvent) => {
      if (!drag || !travel) return;
      const top = drag.top + ((event.clientY - drag.y) * room) / travel;
      // at once: an area that scrolls smoothly (the chat window) would trail behind the pointer
      if (area.scrollTo) area.scrollTo({ top, behavior: 'instant' }); else area.scrollTop = top;
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
      // the bar outlives the area when a panel shows another page (#836): what it showed for this one ends here, as
      // the timer that would have hidden it is gone
      bar.classList.remove('active', 'hover', 'dragging');
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
    <div ref={fades} className="scroll-fades" aria-hidden="true">
      <div><div className="scroll-fade top" /><div className="scroll-fade bottom" /></div>
    </div>
    <div ref={track} className="scrollbar" aria-hidden="true"><div ref={thumb} className="scrollbar-thumb" /></div>
  </>;
}
