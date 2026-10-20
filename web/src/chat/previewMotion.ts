// The preview's picture flies out of its thumbnail and back into it (#194), through a view transition: the thumbnail
// and the preview's picture take the same `view-transition-name` before and after the change, so the browser moves
// and grows one into the other while the rest of the page (the backdrop, the preview's bar) fades. Where the browser
// has no view transitions, or less motion is asked for, the preview opens and closes as it did, with a fade.

export const MEDIA_NAME = 'preview-media';
export const FRAME_NAME = 'preview-frame';

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
 * which `from` turns into what `to` finds after it; `after` runs once it is found, before the new state is taken.
 * Resolves once it has played.
 */
export function fly(from: HTMLElement, change: () => void, to: () => HTMLElement | null | undefined, after?: (target: HTMLElement) => void): Promise<void> {
  let target: HTMLElement | null | undefined = null;
  from.style.viewTransitionName = MEDIA_NAME;
  const transition = document.startViewTransition(() => {
    from.style.viewTransitionName = '';
    change();
    target = to();
    if (target) {
      target.style.viewTransitionName = MEDIA_NAME;
      after?.(target);
    }
  });
  return transition.finished.catch(() => {}).finally(() => {
    from.style.viewTransitionName = '';
    if (target) target.style.viewTransitionName = '';
  });
}

/**
 * The preview's frame (its bar and edges) grows out from under the picture as it flies in, and shrinks back under it
 * as it flies out (#194): the frame's own box goes from the thumbnail's to its place (`open`) or back (`close`), and
 * shows the part of it around the picture, which is on top. Marks the page for the styles that play it (app.css) and
 * returns what takes the marks off once it has played.
 */
export function frameFlight(mode: 'open' | 'close', frame: HTMLElement, media: HTMLElement, thumb: DOMRect): () => void {
  const root = document.documentElement;
  const f = frame.getBoundingClientRect();
  const m = media.getBoundingClientRect();
  const vars: Record<string, string> = {
    '--fly-x': thumb.left + 'px', '--fly-y': thumb.top + 'px', '--fly-w': thumb.width + 'px', '--fly-h': thumb.height + 'px',
    // how far the picture's centre is from the frame's: the frame is shown around it while it is small
    '--frame-ox': (m.left + m.width / 2 - (f.left + f.width / 2)) + 'px',
    '--frame-oy': (m.top + m.height / 2 - (f.top + f.height / 2)) + 'px',
  };
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  root.dataset.flight = mode;
  frame.style.viewTransitionName = FRAME_NAME;
  return () => {
    for (const name of Object.keys(vars)) root.style.removeProperty(name);
    delete root.dataset.flight;
    frame.style.viewTransitionName = '';
  };
}

