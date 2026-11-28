// 几个视图共用、只随服务端和账号（scope.ts）变化的读取，如联系人列表（#402）。
// 同时请求的视图共用一个请求，再次打开的视图直接拿到保留的回答；保留到开始新的作用域、某个视图要求重新读取（重试），
// 或者所在的模块放弃它为止（`forgetKept`，实时消息可能已经改变了它）。失败的读取不保留。
// 离开视图时不取消请求，因为别的视图可能还在等它：离开的视图只是不使用这个回答。
import { onNewScope } from './scope';

const kept = new Map<string, Promise<unknown>>();
onNewScope(() => kept.clear());

/** 以 `key` 保留的回答；没有时用 `read()` 读取，并保留到作用域变化。`fresh` 时重新读取。 */
export function keptRead<T>(key: string, read: () => Promise<T>, fresh = false): Promise<T> {
  const held = kept.get(key) as Promise<T> | undefined;
  if (held && !fresh) return held;
  const promise = read();
  kept.set(key, promise);
  promise.catch(() => { if (kept.get(key) === promise) kept.delete(key); });
  return promise;
}

/** 放弃键符合 `matches` 的读取：下一个请求的视图重新读取，正在等待的仍会拿到回答。 */
export function forgetKept(matches: (key: string) => boolean): void {
  for (const key of [...kept.keys()]) if (matches(key)) kept.delete(key);
}
