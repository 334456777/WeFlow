// The skin's decoration (#832): a layer under the whole page, fixed to the window, that takes no pointer and is
// hidden from assistive technology (app.css .backdrop). A skin draws on it in its own sheet: stickers, a drawing. It
// takes turns among `--backdrop-count` of them, `data-turn` the one up, and changes at each page turn (pageTurn.ts);
// the layer is drawn anew then, so an animation the skin gives it plays at each change. `data-page` counts the same
// way but only the pages, not the details in one, for what the skin sticks on the page (the sticker skin's tapes).
//
// A skin without decorations (a count of 0, as neumorphism) has no layer at all: Chrome composites a layer fixed to
// the window apart from the page, and with it all that is drawn over it, whose text then loses its subpixel smoothing.
import { usePageTurn } from './pageTurn';
import { useSkinOn } from './skin';

/** How many decorations the skin takes turns with (`--backdrop-count`); 0 for none, or a token that is no number. */
export function backdropCount(root: Element = document.documentElement): number {
  const count = Number.parseInt(getComputedStyle(root).getPropertyValue('--backdrop-count'), 10);
  return Number.isFinite(count) && count > 0 ? count : 0;
}

export function Backdrop() {
  // the count is the skin's: read again when another skin is put on
  useSkinOn();
  const { turn, page } = usePageTurn();
  const count = backdropCount();
  return count ? <div key={turn} className="backdrop" data-turn={turn % count} data-page={page % count} aria-hidden="true" /> : null;
}
