---
name: weflow-web-skin
description: 给 WebUI 新增或修改皮肤（web/src/styles/skins/ 和 skin.ts）：登记、样式表的层和选择器、只能设置哪些令牌、外形层与种子、装饰层和换页动画、图片字体与报告导出、手机宽度和减少动画，以及 roles/layers 测试和按皮肤截图对比。做新皮肤、改皮肤的令牌或样式、改外形层、装饰层或换页动画时使用。
---

# 改皮肤

皮肤的设计（三种令牌、角色表、皮肤维度、外形层、装饰层与换页、贴纸皮肤）见 Wiki [WebUI 实现#设计语言](https://github.com/334456777/WeFlow/wiki/WebUI-实现#设计语言)，层和角色的规则见 [CLAUDE.md](../../../CLAUDE.md) 的「WebUI（`web/`）」。页面本身的写法见 [weflow-web-feature](../weflow-web-feature/SKILL.md#样式)。下面以已有的贴纸皮肤（`styles/skins/sticker.css`）为例。

## 登记和样式表

1. 在 `src/skin.ts` 的 `SKINS` 里加一项：`id`、`label`（`msg('…')`，英文加进 `i18n/en.ts`）、`themes`、`sheet: () => import('./styles/skins/<id>.css?inline')`，要外形层时加 `shapes: true`。
2. `id` 不能与角色令牌的前缀相同（如 `card`、`row`）：皮肤的原始令牌以 `--<id>-` 开头，同名时读取那个角色的地方都会被测试当成越界的原始令牌。
3. 样式表是 `styles/skins/<id>.css`，图片和字体放在 `styles/skins/<id>/` 下。全部规则包在 `@layer skin { … }` 里，写在 `:root[data-skin='<id>']` 下。
4. 只有一种主题的皮肤，颜色只写在 `:root[data-skin='<id>']` 下，不写 `[data-theme]`，`themes` 里只写这一种。

## 令牌

- 只设置新拟态或其他样式表、脚本已经给出的令牌（角色令牌表见 Wiki），以及自己的 `--<id>-*`。原始令牌只在自己的样式表里使用，页面不读它。
- 皮肤没有覆盖的令牌沿用新拟态的值：先列出要改的角色和状态，逐个覆盖，不要整份复制新拟态。
- 同一角色各状态的阴影层数相同，状态之间的过渡才连续；没有阴影的状态用 `--shadow-none`。
- 有悬停凹陷的列表，要由各行自己显示悬停时：`--row-hover-track: none`，再给 `--row-shadow-tracked` 和 `--row-shadow-tracked-pressed`。
- 页面背景是图片时，`--fade-bg` 给同一张图，两边都用 `background-attachment: fixed` 对齐视口。
- 颜色用 `oklch()` 按色相随机取色时，彩度取该亮度下所有色相都在 sRGB 色域内的最大值：超出色域的颜色在 Chrome 的裁剪边缘会出现杂色的点。

## 外形层

- `shapes: true` 时，按钮、控件、可选的项和列表行的底色和阴影画在 `::before` 上，外形用各角色的 `-clip`、`-stroke`、`-tilt`、`-drop` 令牌给出。裁剪只加在 `::before` 上，元素本身不裁剪，否则焦点框会被一起裁掉；投影用元素上的 `filter: drop-shadow()`。
- `--seed` 只在这四种角色的元素上有值：用它的规则写在针对这些元素的选择器里，不写在 `:root` 的令牌里（在 `:root` 求值时它没有值）。不给页面上的每个元素声明自定义属性，否则每次样式重算都会变慢。
- 元素带 `rotate` 后，会成为其中 `fixed` 后代的包含块：倾斜的元素里有弹出层时要检查它的位置。

## 装饰层和换页

- `--backdrop-count` 是轮换的装饰张数，按 `.backdrop[data-turn='n']` 选图；换详情时应当留着的东西（如贴在面板上的胶带）按 `data-page` 选。张数为 0 时不渲染装饰层。
- 页面背景只画在 `html` 上，`body` 不加背景，否则会盖住装饰层。
- `--page-in` 是 `animation` 的取值，关键帧写在皮肤的样式表里。要在每次换页时重播的动画，相邻两次用两个名字交替（关键帧相同），名字不变时浏览器不会重播。
- 动画的时长用时长令牌：减少动画时令牌为 0，动画随之不播。动画不阻塞操作。

## 图片和字体

- 样式表里的图片小于 4 KB 时内联成 `data:`，更大的放在 `/ui/assets/` 下，用到时才下载。导出的报告只复制样式表，报告里要显示的图片写成 `url(…?inline)`，不论大小都内联。
- 字体用子集，只保留 ASCII 和界面自身文字用到的字，不含示例数据和测试的文字；子集里没有的字（如联系人的名字）由 `--font-sans` 显示。授权全文放在同一目录，先确认授权允许截取子集。

## 手机宽度

- 倾斜、平移的进入动画会让和窗口一样宽的页面伸出窗口，出现横向滚动：在 `@media (max-width: 640px)` 下换成不倾斜的动画，并给容器加 `overflow-x: clip`。
- 装饰在窄屏上可能被面板挡住，或从标签栏下面露出来：按手机宽度另外摆放或缩小。

## 检查

1. `bun test src/styles src/skin.test.ts`：`roles.test.ts` 检查原始令牌没有越界、只覆盖已有的令牌、读取的变量都有定义；`layers.test.ts` 检查规则都在 `skin` 层的 `:root[data-skin='<id>']` 下。
2. 截图对比，步骤见 [weflow-web-check](../weflow-web-check/SKILL.md#改样式而不该改变外观)：
   - 新皮肤用 `bun run shots <名字> --skin <id>` 截下各场景，浅色、深色各一套，逐张看。一套截图只属于一个皮肤，补截另一个皮肤的场景会失败。
   - 改了 `roles.css`、`tokens.css`、`neumorphism.css` 或外形层的规则时，用默认皮肤在改动前后各截一次比较，确认默认皮肤没有变化；其他已登记的皮肤也各比较一次。
3. 手动看截图覆盖不到的状态：减少动画、高对比度、手机宽度的换页、导出报告（HTML 和 PNG）、刷新后首帧不闪默认皮肤。
4. Wiki：在 [WebUI 实现#设计语言](https://github.com/334456777/WeFlow/wiki/WebUI-实现#设计语言)下给新皮肤加一节，写法见 [weflow-wiki](../weflow-wiki/SKILL.md)。设计语言一节链接的「WeFlow 设计系统」也补上这个皮肤（维护者）。
