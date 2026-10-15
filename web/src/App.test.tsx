import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, onTestFinished, vi } from 'vitest';
import { App } from './App';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
});

async function renderWith(health: Response | Error) {
  vi.stubGlobal('fetch', vi.fn(() => (health instanceof Error ? Promise.reject(health) : Promise.resolve(health.clone()))));
  render(<App />);
  await act(async () => {});
}

describe('App', () => {
  it('shows the workspace and a running backend', async () => {
    await renderWith(new Response(JSON.stringify({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.4' })));
    expect(screen.getByRole('navigation', { name: '主导航' })).toBeTruthy();
    expect(screen.getByRole('region', { name: '操作控制台' })).toBeTruthy();
    expect(screen.queryByText('操作控制台')).toBeNull();
    expect(Array.from(document.querySelectorAll('.card-title'), title => title.textContent)).toEqual(['导出', '当前任务', '搜索', '会话资料', '媒体文件']);
    expect(screen.getByRole('status').textContent).toBe('已连接');
  });

  it('says when there is no backend or an incompatible one', async () => {
    await renderWith(new TypeError('Failed to fetch'));
    expect(screen.getByRole('status').textContent).toBe('未连接');
    cleanup();
    await renderWith(new Response(JSON.stringify({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.3' })));
    expect(screen.getByRole('status').textContent).toBe('版本不兼容');
  });

  it('switches between the conversations and the settings', async () => {
    await renderWith(new TypeError('Failed to fetch'));
    fireEvent.click(screen.getByRole('button', { name: '选择陈屿' }));
    expect(screen.getByText('好的，周一见。', { selector: '.message-bubble' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    expect(screen.getByText('weflow serve')).toBeTruthy();
  });

  it('stretches the export button into progress and cancel, and a cancelled export downloads nothing', async () => {
    // The HTML export waits for the photo; hold that fetch until the export has been cancelled.
    let releasePhoto = () => {};
    const photo = new Promise<void>(resolve => { releasePhoto = resolve; });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === '/health') throw new TypeError('Failed to fetch');
      await photo;
      return new Response('<svg xmlns="http://www.w3.org/2000/svg"/>', { headers: { 'Content-Type': 'image/svg+xml' } });
    }));
    render(<App />);
    await act(async () => {});
    // jsdom has no object URLs; a download needs one.
    const createObjectURL = vi.fn(() => 'blob:export');
    Object.defineProperty(URL, 'createObjectURL', { value: createObjectURL, configurable: true });
    onTestFinished(() => { Reflect.deleteProperty(URL, 'createObjectURL'); });
    const side = () => screen.getByRole('button', { name: /取消导出|再次导出/, hidden: true });
    expect(side().hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    expect(screen.getByRole('button', { name: '正在导出' }).getAttribute('aria-disabled')).toBe('true');
    expect(side().hasAttribute('inert')).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: '取消导出' }));
    await act(async () => { releasePhoto(); await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(screen.getByRole('button', { name: '开始导出' })).toBeTruthy();
    expect(side().hasAttribute('inert')).toBe(true);
    expect(createObjectURL).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: '开始导出' }));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(createObjectURL).toHaveBeenCalledOnce();
    expect(screen.getAllByRole('link', { name: '下载文件' })).toHaveLength(2);
    expect(side().getAttribute('aria-label')).toBe('再次导出');
    expect(side().hasAttribute('inert')).toBe(false);
  });

  it('follows the reduce-motion switch on the root element', async () => {
    await renderWith(new TypeError('Failed to fetch'));
    fireEvent.click(screen.getByRole('button', { name: '设置' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '减少动画' }));
    expect(document.documentElement.classList.contains('reduced-motion')).toBe(true);
    fireEvent.click(screen.getByRole('checkbox', { name: '减少动画' }));
    expect(document.documentElement.classList.contains('reduced-motion')).toBe(false);
  });
});
