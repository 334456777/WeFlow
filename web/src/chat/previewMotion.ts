// The preview's picture flies out of its thumbnail and back into it (#194), through a view transition: the thumbnail
// and the preview's picture take the same `view-transition-name` before and after the change, so the browser moves
// and grows one into the other while the backdrop fades. The page is not taken (shell/PreviewDialog.css): it stays
// live under the flight, so a click meanwhile reaches it and may start another flight, which cuts this one short
// (#844). Where the browser has no view transitions, or less motion is asked for, the preview opens and closes with a
// fade.

/** The `view-transition-name` of the picture in flight. */
export const MEDIA_NAME = 'preview-media';
/** The `view-transition-name` of its frame. */
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

/** The latest flight, and the elements it named: a flight begins by taking the names off the one it cuts short. */
let flights = 0;
let named: HTMLElement[] = [];

/**
 * Runs `change`, which must put the new state on the page before it returns (`flushSync`), in a view transition in
 * which `from` turns into what `to` finds after it; `after` runs once it is found, before the new state is taken.
 * Resolves once it has played: true if it was still the latest flight, false if another began meanwhile, which then
 * has the names and the marks.
 */
export function fly(from: HTMLElement, change: () => void, to: () => HTMLElement | null | undefined, after?: (target: HTMLElement) => void): Promise<boolean> {
  const flight = ++flights;
  for (const element of named) element.style.viewTransitionName = '';
  named = [from];
  from.style.viewTransitionName = MEDIA_NAME;
  const transition = document.startViewTransition(() => {
    from.style.viewTransitionName = '';
    change();
    const target = to();
    if (target && flight === flights) {
      named.push(target);
      target.style.viewTransitionName = MEDIA_NAME;
      after?.(target);
    }
  });
  return transition.finished.catch(() => {}).then(() => {
    if (flight !== flights) return false;
    for (const element of named) element.style.viewTransitionName = '';
    named = [];
    return true;
  });
}

/** The latest frame flight, and the frame it named. */
let frameFlights = 0;
let flownFrame: HTMLElement | null = null;

/**
 * The preview's frame (its bar, its hollow and its footer) grows out from under the picture as it flies in, and
 * shrinks back under it as it flies out (#194): the frame's box goes from the thumbnail's to its place (`open`) or
 * back (`close`), and the frame drawn in it is scaled and moved with the picture (#339), so that its part around the
 * picture stays under the picture all the way and the rest comes out of where the picture comes from. Marks the page
 * for the styles that play it (shell/PreviewDialog.css) and returns what takes the marks off once it has played,
 * unless another frame flight has begun since and set its own.
 */
export function frameFlight(mode: 'open' | 'close', frame: HTMLElement, media: HTMLElement, thumb: DOMRect): () => void {
  const flight = ++frameFlights;
  if (flownFrame && flownFrame !== frame) flownFrame.style.viewTransitionName = '';
  flownFrame = frame;
  const root = document.documentElement;
  const vars = frameVars(frame.getBoundingClientRect(), media.getBoundingClientRect(), thumb);
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  root.dataset.flight = mode;
  frame.style.viewTransitionName = FRAME_NAME;
  return () => {
    if (flight !== frameFlights) return;
    for (const name of Object.keys(vars)) root.style.removeProperty(name);
    delete root.dataset.flight;
    frame.style.viewTransitionName = '';
    flownFrame = null;
  };
}

type Box = Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>;

/**
 * The boxes of a frame flight, for shell/PreviewDialog.css: the thumbnail's (`--fly-*`), the frame's size (`--frame-w`, `--frame-h`),
 * and the scale and shift that put the frame's picture `media` on the thumbnail (`--frame-s*`, `--frame-t*`), from the
 * frame's own corner.
 */
export function frameVars(frame: Box, media: Box, thumb: Box): Record<string, string> {
  const sx = media.width > 0 ? thumb.width / media.width : 1;
  const sy = media.height > 0 ? thumb.height / media.height : 1;
  return {
    '--fly-x': thumb.left + 'px', '--fly-y': thumb.top + 'px', '--fly-w': thumb.width + 'px', '--fly-h': thumb.height + 'px',
    '--frame-w': frame.width + 'px', '--frame-h': frame.height + 'px',
    '--frame-sx': String(sx), '--frame-sy': String(sy),
    '--frame-tx': -(media.left - frame.left) * sx + 'px', '--frame-ty': -(media.top - frame.top) * sy + 'px',
  };
}
