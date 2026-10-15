import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
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
});
