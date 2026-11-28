// 预览的图片通过视图过渡（view transition）从缩略图飞出、再飞回（#194）：
// 缩略图和预览里的图片在变化前后取同一个 `view-transition-name`，浏览器因此把前者移动、放大成后者，同时背景淡入或淡出。
// 页面本身不参与截取（shell/PreviewDialog.css）：飞行期间它照常运行，这时的点击会落到页面上，并可能开始另一次飞行，
// 打断这一次（#844）。浏览器不支持视图过渡，或者要求减少动画时，预览用淡入淡出的方式打开和关闭。

/** 飞行中的图片的 `view-transition-name`。 */
export const MEDIA_NAME = 'preview-media';
/** 图片的框的 `view-transition-name`。 */
export const FRAME_NAME = 'preview-frame';

/** 打开预览的控件里的缩略图：先找聚焦的（用键盘时，以及多数浏览器里点击时），再找最后按下的；只取已经加载完的，
 * 因为图片到达之前，预览按它的形状确定大小。 */
export function thumbnailOf(...candidates: (Element | null | undefined)[]): HTMLImageElement | null {
  for (const element of candidates) {
    if (!(element instanceof Element)) continue;
    const img = element instanceof HTMLImageElement ? element : element.closest('button, a, [role="button"]')?.querySelector('img');
    if (img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0 && img.naturalHeight > 0) return img;
  }
  return null;
}

/** 图片能否飞行：浏览器支持，并且没有要求减少动画。 */
export function canFly(reduced: boolean): boolean {
  return !reduced && typeof document !== 'undefined' && typeof document.startViewTransition === 'function';
}

/** `element` 是否仍在页面上、并且至少有一部分在屏幕内，可以飞回去。 */
export function inSight(element: Element): boolean {
  if (!element.isConnected) return false;
  const box = element.getBoundingClientRect();
  return box.width > 0 && box.bottom > 0 && box.right > 0 && box.top < window.innerHeight && box.left < window.innerWidth;
}

/** 最近一次飞行，以及它设了名字的元素：新的飞行开始时，先去掉被它打断的那次飞行设的名字。 */
let flights = 0;
let named: HTMLElement[] = [];

/**
 * 在视图过渡里运行 `change`（它必须在返回之前把新状态放到页面上，即 `flushSync`），
 * 让 `from` 变成之后 `to` 找到的元素；找到后、截取新状态之前运行 `after`。
 * 播放完后兑现为 true（仍是最近一次飞行），或者 false（这期间开始了另一次飞行，名字和标记归它）。
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

/** 最近一次框的飞行，以及它设了名字的框。 */
let frameFlights = 0;
let flownFrame: HTMLElement | null = null;

/**
 * 预览的框（栏、凹陷和底栏）在图片飞进来时从它下面长出来，飞出去时缩回它下面（#194）：
 * 框的位置和大小从缩略图的变到自己的（`open`），或者反过来（`close`）；框的截图与图片一起缩放和移动（#339），
 * 它在图片周围的部分一路都在图片下面，其余部分从图片飞来的地方出现。
 * 给页面加上播放用的样式（shell/PreviewDialog.css）所需的标记，并返回播放完后去掉标记的函数；
 * 之后又开始了另一次框的飞行、设了它自己的标记时，这个函数什么都不做。
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
 * 框的飞行用到的位置和大小，供 shell/PreviewDialog.css 使用：缩略图的（`--fly-*`）、框的大小（`--frame-w`、
 * `--frame-h`），以及从框自己的左上角算起、把框的截图里的 `media` 放到缩略图上的缩放和位移（`--frame-s*`、
 * `--frame-t*`）。
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
