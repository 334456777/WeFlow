// The hover hollow that slides (#232): a group of clickable items (the navigation, a list) shares one shallow
// hollow. When the pointer crosses into another item, the hollow slides over from the item it was on, rather than
// one fading out and the other in; it does not follow the pointer itself. Coming into the group it appears where the
// pointer is, and leaving it fades away. Keyboard focus (when it shows) moves it too.
import { useEffect, useRef } from 'react';

/** After a scroll the rows move under the pointer: for this long the hollow jumps instead of sliding. */
const SCROLL_QUIET_MS = 150;

/** Put it first in the group (its parent), which it positions; the hollow scrolls with the group's items. */
export function HoverTrack({ items, quiet = '.selected, .active' }: {
  /** The items, as a selector. */
  items: string;
  /** Items already sunk (the selection): the hollow fades out over them instead of doubling their shade. */
  quiet?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const hollow = ref.current;
    const group = hollow?.parentElement;
    if (!group || !hollow) return;
    group.classList.add('hover-track-host');
    let current: HTMLElement | null = null;
    let hovered = false;
    let scrolledAt = 0;

    const place = (item: HTMLElement, slide: boolean) => {
      const box = group.getBoundingClientRect();
      const at = item.getBoundingClientRect();
      const x = at.left - box.left - group.clientLeft + group.scrollLeft;
      const y = at.top - box.top - group.clientTop + group.scrollTop;
      if (!slide) hollow.style.transition = 'none';
      hollow.style.width = at.width + 'px';
      hollow.style.height = at.height + 'px';
      hollow.style.transform = `translate(${x}px, ${y}px)`;
      hollow.style.borderRadius = getComputedStyle(item).borderRadius;
      if (!slide) { void hollow.offsetWidth; hollow.style.transition = ''; }
    };
    const show = (item: HTMLElement) => {
      // it slides only from where it is showing; coming in from outside (or right after a scroll) it appears in place
      const slide = (hollow.dataset.state === 'on' || hollow.dataset.state === 'quiet') && Date.now() - scrolledAt > SCROLL_QUIET_MS;
      place(item, slide);
      current = item;
      hollow.dataset.state = item.matches(quiet) ? 'quiet' : 'on';
    };
    const hide = () => {
      current = null;
      if (hollow.dataset.state) hollow.dataset.state = 'off';
    };
    const itemOf = (target: EventTarget | null) => {
      const item = target instanceof Element ? target.closest<HTMLElement>(items) : null;
      return item && group.contains(item) ? item : null;
    };

    const over = (event: PointerEvent) => {
      if (event.pointerType === 'touch') return;
      hovered = true;
      // in the gaps between items it stays where it is, so the next item still slides it on
      const item = itemOf(event.target);
      if (item && item !== current) show(item);
    };
    const leave = () => {
      hovered = false;
      const focused = itemOf(document.activeElement);
      if (focused?.matches(':focus-visible')) show(focused);
      else hide();
    };
    const focusIn = (event: FocusEvent) => {
      const item = itemOf(event.target);
      if (item?.matches(':focus-visible') && !hovered) show(item);
    };
    const focusOut = () => { if (!hovered) hide(); };
    const scrolled = () => { scrolledAt = Date.now(); };
    // the item changed (a row taken by another conversation, the sidebar folding): follow it at once
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => { if (current?.isConnected) place(current, false); else if (current) hide(); });
    resize?.observe(group);
    const changed = new MutationObserver(() => { if (current) hollow.dataset.state = !current.isConnected ? 'off' : current.matches(quiet) ? 'quiet' : 'on'; });
    changed.observe(group, { subtree: true, attributes: true, attributeFilter: ['class', 'aria-pressed', 'aria-current'], childList: true });

    group.addEventListener('pointerover', over);
    group.addEventListener('pointerleave', leave);
    group.addEventListener('focusin', focusIn);
    group.addEventListener('focusout', focusOut);
    window.addEventListener('scroll', scrolled, { capture: true, passive: true });
    return () => {
      group.classList.remove('hover-track-host');
      group.removeEventListener('pointerover', over);
      group.removeEventListener('pointerleave', leave);
      group.removeEventListener('focusin', focusIn);
      group.removeEventListener('focusout', focusOut);
      window.removeEventListener('scroll', scrolled, { capture: true });
      resize?.disconnect();
      changed.disconnect();
    };
  }, [items, quiet]);

  return <div ref={ref} className="hover-hollow" aria-hidden="true" />;
}
