import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { reportHtml, reportSvg, withoutPictures } from './reportFiles';
import { SaveReport } from './SaveReport';

afterEach(() => {
  cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.innerHTML = '';
  for (const name of ['style', 'data-theme', 'data-skin']) document.documentElement.removeAttribute(name);
});

function report(): HTMLElement {
  const article = document.createElement('article');
  article.className = 'report';
  article.setAttribute('aria-label', '2025 年的年度报告');
  article.innerHTML = '<img class="avatar" src="https://wx.qlogo.cn/x" data-initial="林"><strong>12,345</strong><span>a < b & "c"</span>';
  document.body.append(article);
  return article;
}

describe('saving a report', () => {
  it('leaves the pictures out, their initials in their place', () => {
    const copy = withoutPictures(report());
    expect(copy.querySelector('img')).toBeNull();
    expect(copy.querySelector('.avatar-initial')?.textContent).toBe('林');
    expect(document.querySelector('article img')).toBeTruthy();
  });

  it('makes a page of its own in the colours of the theme in use', () => {
    document.documentElement.style.setProperty('--page-bg', '#25282c');
    const html = reportHtml(report(), 'WeFlow <2025>');
    expect(html).toMatch(/^<!doctype html>/);
    expect(html).toContain('<title>WeFlow &lt;2025&gt;</title>');
    expect(html).toContain('--page-bg: #25282c');
    expect(html).toContain('<article class="report" aria-label="2025 年的年度报告">');
    expect(html).not.toContain('qlogo');
    expect(html).toContain('a &lt; b &amp; "c"');
  });

  it('names the theme and the skin in use on the copy’s root, where a skin’s rules look for them (#830)', () => {
    Object.assign(document.documentElement.dataset, { theme: 'dark', skin: 'neumorphism' });
    expect(reportHtml(report(), 'x')).toContain('<html lang="zh-CN" data-theme="dark" data-skin="neumorphism"><head>');
    expect(reportSvg(report(), 600).svg).toMatch(/^<svg xmlns="http:\/\/www.w3.org\/2000\/svg" data-theme="dark" data-skin="neumorphism" width="600"/);
  });

  it('draws it as an SVG of XHTML, the size it takes', () => {
    const { svg, height } = reportSvg(report(), 600);
    expect(svg).toMatch(/^<svg xmlns="http:\/\/www.w3.org\/2000\/svg" width="600" height="\d+"><foreignObject/);
    expect(svg).toContain('<div xmlns="http://www.w3.org/1999/xhtml"');
    expect(svg).not.toContain('qlogo');
    expect(height).toBeGreaterThan(0);
    expect(document.body.children).toHaveLength(1);
  });

  it('hands the page to the browser named after the report, and says when it is not there yet', async () => {
    const saved = downloads();
    let shown: HTMLElement | null = null;
    render(<SaveReport report="annual:2025" find={() => shown} />);
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    expect(screen.getByRole('alert').textContent).toBe('报告还没有生成好。');
    shown = report();
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    await act(async () => {});
    expect(saved).toEqual(['WeFlow 2025 年的年度报告.html']);
    expect(screen.getByText(DONE)).toBeTruthy();
  });

  it('saves nothing while no report is shown, and lets go of what it said once another one is (#1077)', async () => {
    const saved = downloads();
    const article = report();
    const card = (key: string | null) => <SaveReport report={key} find={() => (key ? article : null)} />;
    const disabled = () => ['保存为图片', '保存为网页'].map(name => (screen.getByRole('button', { name }) as HTMLButtonElement).disabled);
    const { rerender } = render(card(null));
    expect(disabled()).toEqual([true, true]);
    rerender(card('annual:2025'));
    expect(disabled()).toEqual([false, false]);
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    await act(async () => {});
    expect(screen.getByText(DONE)).toBeTruthy();
    // the dual report, no friend picked yet: nothing to save, and nothing said of the annual one
    rerender(card(null));
    expect(disabled()).toEqual([true, true]);
    expect(screen.queryByText(DONE)).toBeNull();
    // back on it, what was said is not said again
    rerender(card('annual:2025'));
    expect(screen.queryByText(DONE)).toBeNull();
    // another year
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    await act(async () => {});
    expect(screen.getByText(DONE)).toBeTruthy();
    rerender(card('annual:2024'));
    expect(screen.queryByText(DONE)).toBeNull();
    expect(saved).toHaveLength(2);
  });
});

const DONE = '已交给浏览器下载。头像换成了名字的第一个字。';

/** The names of the files handed to the browser from now on. */
function downloads(): string[] {
  const saved: string[] = [];
  // spied on, not assigned: `URL` is one object for every test file, and `restoreAllMocks` puts spies back (#931)
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => 'blob:x');
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) { saved.push(this.download); });
  return saved;
}
