import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { reportHtml, reportSvg, withoutPictures } from './reportFiles';
import { SaveReport } from './SaveReport';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.innerHTML = ''; document.documentElement.removeAttribute('style'); });

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
    document.documentElement.style.setProperty('--surface', '#25282c');
    const html = reportHtml(report(), 'WeFlow <2025>');
    expect(html).toMatch(/^<!doctype html>/);
    expect(html).toContain('<title>WeFlow &lt;2025&gt;</title>');
    expect(html).toContain('--surface: #25282c');
    expect(html).toContain('<article class="report" aria-label="2025 年的年度报告">');
    expect(html).not.toContain('qlogo');
    expect(html).toContain('a &lt; b &amp; "c"');
  });

  it('draws it as an SVG of XHTML, the size it takes', () => {
    const { svg, height } = reportSvg(report(), 600);
    expect(svg).toMatch(/^<svg xmlns="http:\/\/www.w3.org\/2000\/svg" width="600" height="\d+"><foreignObject/);
    expect(svg).toContain('<div xmlns="http://www.w3.org/1999/xhtml"');
    expect(svg).not.toContain('qlogo');
    expect(height).toBeGreaterThan(0);
    expect(document.body.children).toHaveLength(1);
  });

  it('hands the page to the browser named after the report, and says when there is none yet', async () => {
    const saved: string[] = [];
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: vi.fn(() => 'blob:x'), revokeObjectURL: vi.fn() }));
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) { saved.push(this.download); });
    let shown: HTMLElement | null = null;
    render(<SaveReport find={() => shown} />);
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    expect(screen.getByRole('alert').textContent).toBe('报告还没有生成好。');
    shown = report();
    fireEvent.click(screen.getByRole('button', { name: '保存为网页' }));
    await act(async () => {});
    expect(saved).toEqual(['WeFlow 2025 年的年度报告.html']);
    expect(screen.getByText('已交给浏览器下载。头像换成了名字的第一个字。')).toBeTruthy();
  });
});
