// The preview's picture flies out of its thumbnail and back into it (#194), through a view transition: the thumbnail
// and the preview's picture take the same `view-transition-name` before and after the change, so the browser moves
// and grows one into the other while the rest of the page (the backdrop, the preview's bar) fades. Where the browser
// has no view transitions, or less motion is asked for, the preview opens and closes as it did, with a fade.

export const MEDIA_NAME = 'preview-media';

/** The thumbnail of the control a preview opens from: the one focused (keys, and clicks in most browsers), else the
 * one last pressed; only once it has loaded, as its shape sizes the preview before the picture comes. */
export function thumbnailOf(...candidates: (Element | null | undefined)[]): HTMLImageElement | null {
  for (const element of candidates) {
    if (!(element instanceof Element)) continue;
    const img = element instanceof HTMLImageElement ? element : element.closest('button, a, [role="button"]')?.querySelector('img');
    if (img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0 && img.naturalHeight > 0) return img;
  }
  return null;
}

/** Whether the picture may fly: the browser can, and nobody asked for less motion. */
export function canFly(reduced: boolean): boolean {
  return !reduced && typeof document !== 'undefined' && typeof document.startViewTransition === 'function';
}

/** Whether `element` is still on the page and at least partly on screen, to fly back into. */
export function inSight(element: Element): boolean {
  if (!element.isConnected) return false;
  const box = element.getBoundingClientRect();
  return box.width > 0 && box.bottom > 0 && box.right > 0 && box.top < window.innerHeight && box.left < window.innerWidth;
}

/**
 * Runs `change`, which must put the new state on the page before it returns (`flushSync`), in a view transition in
 * which `from` turns into what `to` finds after it; resolves once it has played.
 */
export function fly(from: HTMLElement, change: () => void, to: () => HTMLElement | null | undefined): Promise<void> {
  let target: HTMLElement | null | undefined = null;
  from.style.viewTransitionName = MEDIA_NAME;
  const transition = document.startViewTransition(() => {
    from.style.viewTransitionName = '';
    change();
    target = to();
    if (target) target.style.viewTransitionName = MEDIA_NAME;
  });
  return transition.finished.catch(() => {}).finally(() => {
    from.style.viewTransitionName = '';
    if (target) target.style.viewTransitionName = '';
  });
}
