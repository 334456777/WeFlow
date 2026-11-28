// 这个页面此刻能否保持它的长连接（#799）。走 HTTP/1.1 时 Chrome 对一个服务端最多开六个连接，
// 浏览器的所有标签页共用这些连接；每个页面都保持一个事件流（backend.ts，以及作业运行期间的 useJobs.ts）。
// 不可见的页面放开它的事件流，让可见的页面总有连接可用；页面一旦重新可见就立刻重新连上。
//
// - 隐藏（切到别的标签页、窗口最小化或被挡住）：`HIDDEN_MS` 之后放开；重新可见：立刻连回
// - `pagehide`（进入往返缓存或页面关闭）和 `freeze`：立刻放开；`pageshow` 和 `resume`：按页面的可见状态处理

/** 隐藏的页面保持事件流的时长，这样切到别的标签页看一眼再回来不必重新连接。 */
export const HIDDEN_MS = 10_000;

let here = true;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<(here: boolean) => void>();

function set(next: boolean): void {
  if (next === here) return;
  here = next;
  for (const listener of [...listeners]) listener(next);
}

const judge = () => {
  clearTimeout(timer);
  timer = undefined;
  if (document.visibilityState === 'visible') set(true);
  else if (here) timer = setTimeout(() => set(false), HIDDEN_MS);
};

const leave = () => {
  clearTimeout(timer);
  timer = undefined;
  set(false);
};

function watch(): void {
  document.addEventListener('visibilitychange', judge);
  document.addEventListener('freeze', leave);
  document.addEventListener('resume', judge);
  window.addEventListener('pagehide', leave);
  window.addEventListener('pageshow', judge);
  // 在后台打开的页面从现在起算作隐藏
  here = true;
  judge();
}

function unwatch(): void {
  document.removeEventListener('visibilitychange', judge);
  document.removeEventListener('freeze', leave);
  document.removeEventListener('resume', judge);
  window.removeEventListener('pagehide', leave);
  window.removeEventListener('pageshow', judge);
  clearTimeout(timer);
  timer = undefined;
  here = true;
}

/** 页面此刻能否保持它的事件流。 */
export function present(): boolean {
  return here;
}

/** 页面每次离开或回来时调用 `listener`；返回停止监听的函数。 */
export function onPresence(listener: (here: boolean) => void): () => void {
  if (!listeners.size) watch();
  listeners.add(listener);
  return () => {
    if (listeners.delete(listener) && !listeners.size) unwatch();
  };
}
