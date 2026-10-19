// A report saved from the browser (#208): the report as shown, in the page's styles and the colours of the theme in
// use, as a page of its own (HTML) or a picture (PNG). The CLI has no such output; the desktop app takes a screenshot.
// Pictures are left out (avatars as their initials): WeChat's avatar links expire, and a picture from elsewhere would
// stop the canvas from being read.

const XHTML = 'http://www.w3.org/1999/xhtml';

/** Every rule of the page's own style sheets. */
export function pageCss(): string {
  return Array.from(document.styleSheets, sheet => {
    try { return Array.from(sheet.cssRules, rule => rule.cssText).join('\n'); } catch { return ''; }
  }).join('\n');
}

/** The custom properties as they are now — the theme in use — to fix on the copy, which no media query may change. */
export function themeVariables(): string {
  const style = getComputedStyle(document.documentElement);
  const pairs: string[] = [];
  for (let i = 0; i < style.length; i++) {
    const name = style[i];
    if (name.startsWith('--')) pairs.push(name + ': ' + style.getPropertyValue(name).trim());
  }
  return pairs.join('; ');
}

/** A copy of the report with each picture swapped for its initial. */
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

/** Styles of the copy itself: the page scrolls as a document, the report sits in a column on the surface. */
const OWN_CSS = 'html, body { height: auto; overflow: auto; } body { padding: 24px 16px; } .report-export { max-width: 880px; margin: 0 auto; } .report-export .report { margin-top: 0; }';

/** The report as a page of its own. */
export function reportHtml(report: HTMLElement, title: string): string {
  const copy = withoutPictures(report);
  return '<!doctype html>\n<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">'
    + '<title>' + escape(title) + '</title><style>' + pageCss().replaceAll('</style', '<\\/style') + '\n' + OWN_CSS + '</style></head>'
    + '<body style="' + escape(themeVariables()).replaceAll('"', '&quot;') + '"><main class="report-export">' + copy.outerHTML + '</main></body></html>\n';
}

/** The report as an SVG that draws it in a foreignObject, `width` CSS pixels wide. */
export function reportSvg(report: HTMLElement, width: number): { svg: string; height: number } {
  const copy = withoutPictures(report);
  const frame = document.createElementNS(XHTML, 'div') as HTMLDivElement;
  frame.setAttribute('style', themeVariables() + '; width: ' + width + 'px; padding: 24px; box-sizing: border-box; background: var(--surface); color: var(--fg); font-family: var(--font-sans); font-size: var(--text-md); line-height: var(--leading-md)');
  const style = document.createElementNS(XHTML, 'style');
  style.textContent = pageCss() + '\n.report { margin-top: 0; }';
  frame.append(style, copy);
  // measured as laid out in the page, at the width it will have
  const probe = document.createElement('div');
  probe.setAttribute('style', 'position: fixed; left: -100000px; top: 0; visibility: hidden');
  probe.append(frame);
  document.body.append(probe);
  const height = Math.ceil(frame.getBoundingClientRect().height) || report.scrollHeight + 48;
  probe.remove();
  const body = new XMLSerializer().serializeToString(frame);
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '"><foreignObject x="0" y="0" width="100%" height="100%">' + body + '</foreignObject></svg>';
  return { svg, height };
}

/** The report as a PNG, at twice the size for sharp text. */
export async function reportPng(report: HTMLElement, width = Math.min(880, Math.max(360, report.clientWidth)) + 48): Promise<Blob> {
  const { svg, height } = reportSvg(report, width);
  const picture = new Image();
  picture.decoding = 'sync';
  await new Promise<void>((resolve, reject) => {
    picture.onload = () => resolve();
    picture.onerror = () => reject(new Error('没能画出这份报告。'));
    picture.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
  });
  const scale = 2;
  const canvas = document.createElement('canvas');
  canvas.width = width * scale;
  canvas.height = height * scale;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('浏览器不能生成图片。');
  context.scale(scale, scale);
  context.drawImage(picture, 0, 0, width, height);
  return new Promise((resolve, reject) => canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('浏览器不能生成图片。'))), 'image/png'));
}
