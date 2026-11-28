// 在浏览器里保存报告（#208）：按显示的样子，用页面的样式和当前主题的颜色，保存成单独的网页（HTML）或图片（PNG）。
// 命令行没有这样的输出，所以由网页生成。图片不保存（头像换成名字的第一个字）：微信的头像链接会过期，
// 而来自别处的图片会使画布（canvas）不能再被读出。
import { locale, t } from '../i18n';

const XHTML = 'http://www.w3.org/1999/xhtml';

/** 页面的样式表里的全部规则；读不了规则的样式表（如跨域的）跳过。 */
export function pageCss(): string {
  return Array.from(document.styleSheets, sheet => {
    try { return Array.from(sheet.cssRules, rule => rule.cssText).join('\n'); } catch { return ''; }
  }).join('\n');
}

/** 自定义属性此刻的值（即当前的主题），写死在副本上，媒体查询就改不了它们。 */
export function themeVariables(): string {
  const style = getComputedStyle(document.documentElement);
  const pairs: string[] = [];
  for (let i = 0; i < style.length; i++) {
    const name = style[i];
    if (name.startsWith('--')) pairs.push(name + ': ' + style.getPropertyValue(name).trim());
  }
  return pairs.join('; ');
}

/** 报告的副本，每张图片换成名字的第一个字（图片的 `data-initial`）。 */
export function withoutPictures(report: HTMLElement): HTMLElement {
  const copy = report.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('img').forEach(img => {
    const initial = document.createElement('span');
    initial.className = 'avatar avatar-initial';
    initial.setAttribute('aria-hidden', 'true');
    initial.textContent = img.dataset.initial ?? '';
    img.replaceWith(initial);
  });
  return copy;
}

const escape = (text: string) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

/** 当前的主题和皮肤（#830），写成副本根元素的属性，这样写在 `:root[data-skin]` 下的皮肤规则在副本里同样生效。 */
function looks(): string {
  const { theme, skin } = document.documentElement.dataset;
  return [['data-theme', theme], ['data-skin', skin]]
    .filter(([, value]) => value)
    .map(([name, value]) => ' ' + name + '="' + escape(value!).replaceAll('"', '&quot;') + '"').join('');
}

/** 副本自己的样式：整个页面像普通文档一样滚动，报告排在页面中间的一栏里。 */
const OWN_CSS = 'html, body { height: auto; overflow: auto; } body { padding: 24px 16px; } .report-export { max-width: 880px; margin: 0 auto; } .report-export .report { margin-top: 0; }';

/** 把报告做成单独的网页，返回 HTML 文本。 */
export function reportHtml(report: HTMLElement, title: string): string {
  const copy = withoutPictures(report);
  return '<!doctype html>\n<html lang="' + locale() + '"' + looks() + '><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">'
    + '<title>' + escape(title) + '</title><style>' + pageCss().replaceAll('</style', '<\\/style') + '\n' + OWN_CSS + '</style></head>'
    + '<body style="' + escape(themeVariables()).replaceAll('"', '&quot;') + '"><main class="report-export">' + copy.outerHTML + '</main></body></html>\n';
}

/** 把报告画在 SVG 的 `foreignObject` 里，宽 `width` 个 CSS 像素；同时返回量出的高度。 */
export function reportSvg(report: HTMLElement, width: number): { svg: string; height: number } {
  const copy = withoutPictures(report);
  const frame = document.createElementNS(XHTML, 'div') as HTMLDivElement;
  frame.setAttribute('style', themeVariables() + '; width: ' + width + 'px; padding: 24px; box-sizing: border-box; background: var(--page-bg); color: var(--fg); font-family: var(--font-sans); font-size: var(--text-md); line-height: var(--leading-md)');
  const style = document.createElementNS(XHTML, 'style');
  style.textContent = pageCss() + '\n.report { margin-top: 0; }';
  frame.append(style, copy);
  // 放进页面、按图片的宽度排版后量出高度
  const probe = document.createElement('div');
  probe.setAttribute('style', 'position: fixed; left: -100000px; top: 0; visibility: hidden');
  probe.append(frame);
  document.body.append(probe);
  const height = Math.ceil(frame.getBoundingClientRect().height) || report.scrollHeight + 48;
  probe.remove();
  const body = new XMLSerializer().serializeToString(frame);
  // SVG 是这张图片的根元素，也就是 `:root` 所指的元素，所以主题和皮肤的属性写在它上面
  const svg = '<svg xmlns="http://www.w3.org/2000/svg"' + looks() + ' width="' + width + '" height="' + height + '"><foreignObject x="0" y="0" width="100%" height="100%">' + body + '</foreignObject></svg>';
  return { svg, height };
}

/** 把报告画成 PNG，按两倍的尺寸画，文字更清晰。 */
export async function reportPng(report: HTMLElement, width = Math.min(880, Math.max(360, report.clientWidth)) + 48): Promise<Blob> {
  const { svg, height } = reportSvg(report, width);
  const picture = new Image();
  picture.decoding = 'sync';
  await new Promise<void>((resolve, reject) => {
    picture.onload = () => resolve();
    picture.onerror = () => reject(new Error(t('没能画出这份报告。')));
    picture.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
  });
  const scale = 2;
  const canvas = document.createElement('canvas');
  canvas.width = width * scale;
  canvas.height = height * scale;
  const context = canvas.getContext('2d');
  if (!context) throw new Error(t('浏览器不能生成图片。'));
  context.scale(scale, scale);
  context.drawImage(picture, 0, 0, width, height);
  return new Promise((resolve, reject) => canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error(t('浏览器不能生成图片。')))), 'image/png'));
}
