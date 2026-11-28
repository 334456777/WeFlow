// 皮肤的装饰层（#832）：铺在整个页面下面、固定在窗口上的一层，不接收指针，对辅助技术隐藏（app.css 的 .backdrop）。
// 皮肤在自己的样式表里往上面画装饰，如贴纸、一幅画。装饰在 `--backdrop-count` 个之间轮换，`data-turn` 是当前的一个，
// 每次换页（pageTurn.ts）时换一个；这时这一层重新画出，所以皮肤给它的动画在每次换装饰时都会播放。
// `data-page` 同样计数，但只数换页、不数同一页里换详情，供皮肤贴在页面上的东西使用（贴纸皮肤的胶带）。
//
// 没有装饰的皮肤（个数为 0，如新拟态）完全没有这一层：Chrome 会把固定在窗口上的层与页面分开合成，
// 画在它上面的所有内容也一起分开合成，这些内容里的文字就失去了亚像素抗锯齿。
import { usePageTurn } from './pageTurn';
import { useSkinOn } from './skin';

/** 皮肤轮换的装饰个数（`--backdrop-count`）；没有装饰或者令牌不是数字时为 0。 */
export function backdropCount(root: Element = document.documentElement): number {
  const count = Number.parseInt(getComputedStyle(root).getPropertyValue('--backdrop-count'), 10);
  return Number.isFinite(count) && count > 0 ? count : 0;
}

/** 皮肤的装饰层；皮肤没有装饰时不渲染。 */
export function Backdrop() {
  // 个数由皮肤决定：换了皮肤后重新读取
  useSkinOn();
  const { turn, page } = usePageTurn();
  const count = backdropCount();
  return count ? <div key={turn} className="backdrop" data-turn={turn % count} data-page={page % count} aria-hidden="true" /> : null;
}
