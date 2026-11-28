// 网页显示谁的记录，只按连接的状态判断（#876、#879）。这里单独成一个模块，不加载连接的任何代码，
// 只需要它的模块（scope.ts，测试的 preload 在 DOM 就绪之前就加载它）因此不会加载连接的代码。
import type { Backend } from './backend';

/**
 * 网页是否显示服务端的记录：服务端就绪时显示；就绪过的服务端断开后也显示，显示的是断开前读到的记录。
 */
export const holdsRecords = (backend: Backend): boolean =>
  backend.state === 'ready' || (backend.state === 'offline' && backend.held === true);

/**
 * 网页显示谁的记录：服务端的（`holdsRecords`）；尚无服务端回答过，或者连上的服务端不适合这个网页时，
 * 显示示例数据（src/mock，#879）；正在寻找服务端，或者服务端拒绝这个标签页的 token 时，什么都不显示。
 */
export function recordsOf(backend: Backend): 'server' | 'sample' | null {
  if (holdsRecords(backend)) return 'server';
  return backend.state === 'offline' || backend.state === 'incompatible' ? 'sample' : null;
}
