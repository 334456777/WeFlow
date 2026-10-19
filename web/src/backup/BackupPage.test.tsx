import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { backupParams, backupParts } from '../api/backup';
import type { Job } from '../api/jobs';
import type { JobsState } from '../jobs/useJobs';
import { BackupPage } from './BackupPage';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const job = (over: Partial<Job>): Job => ({
  id: 'j1', status: 'succeeded', command: ['weflow', '--json', '--progress', 'backup', 'create'], createdAt: Date.UTC(2026, 9, 19, 12),
  startedAt: null, finishedAt: null, exitCode: 0, httpStatus: 200, progress: null, ...over,
});
const NONE: JobsState = { jobs: [], labels: {} };

describe('backup', () => {
  it('groups what a backup holds the way `backup create` collects it, and leaves out what is unticked', () => {
    expect(backupParts([
      { path: 'db_storage/message/message_0.db', sha256: '', size: 300 },
      { path: 'db_storage\\contact\\contact.db', sha256: '', size: 100 },
      { path: 'FileStorage/Image2/2026-10/a.dat', sha256: '', size: 20 },
      { path: 'FileStorage/Image/2026-10/b.dat', sha256: '', size: 30 },
      { path: 'emojis/c.gif', sha256: '', size: 5 },
      { path: 'manifest.extra', sha256: '', size: 1 },
    ])).toEqual([
      { label: '数据库', files: 2, bytes: 400 },
      { label: '图片', files: 2, bytes: 50 },
      { label: '表情', files: 1, bytes: 5 },
      { label: '其他', files: 1, bytes: 1 },
    ]);
    expect(JSON.parse(JSON.stringify(backupParams({ images: true, voice: true, emojis: true })))).toEqual({});
    expect(backupParams({ images: false, voice: true, emojis: false })).toEqual({ 'no-images': true, 'no-voice': undefined, 'no-emojis': true });
  });

  it('makes a backup as a job that keeps the file, without the parts unticked', async () => {
    const calls: { url: string; body: unknown; prefer: string | undefined }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ url, body: JSON.parse(String(init?.body)), prefer: (init?.headers as Record<string, string>)?.Prefer });
      return json({ success: true, data: job({ status: 'queued' }) }, 202);
    }));
    render(<BackupPage jobs={NONE} />);
    fireEvent.click(screen.getByRole('checkbox', { name: '语音' }));
    fireEvent.click(screen.getByRole('button', { name: '创建备份' }));
    await act(async () => {});
    expect(calls).toEqual([{ url: '/backup/create?return=file', body: { 'no-voice': true }, prefer: 'respond-async' }]);
    expect(screen.getByText('已开始备份，进度在任务中心和下面的列表里。')).toBeTruthy();
  });

  it('lists the backups this tab made, reads one, and restores it into a folder only once one is given', async () => {
    const jobs: JobsState = {
      jobs: [
        job({ id: 'j2', status: 'running', progress: { stage: '复制文件', current: 3, total: 10 } }),
        job({ file: { name: 'backup.zip', size: 2048 }, result: { success: true, data: { out: '/srv/cache/serve-files/x/backup.zip' } } }),
        job({ id: 'j3', command: ['weflow', 'export', 'sessions'] }),
      ],
      labels: { j1: '备份数据库', j2: '备份数据库', j3: '导出' },
    };
    const calls: { method: string; url: string; body?: unknown }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ method: init?.method ?? 'GET', url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      return url.startsWith('/backup/inspect')
        ? json({ success: true, data: { version: '1', created_at: 1_760_000_000, wxid: 'wxid_demo', weflow_version: '5.0.0', entries: [{ path: 'db_storage/a.db', sha256: '', size: 1024 }] } })
        : json({ success: true, data: job({ id: 'j4', status: 'queued' }) }, 202);
    }));
    render(<BackupPage jobs={jobs} />);
    const rows = screen.getByRole('list', { name: '这个标签页创建的备份' }).querySelectorAll('li');
    expect(rows).toHaveLength(2);
    expect(rows[1].textContent).toMatch(/已完成 · backup.zip · 2.0 KB下载查看内容$/);
    expect(rows[1].querySelector('a')?.getAttribute('download')).toBe('backup.zip');
    expect(rows[0].querySelector('a')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '查看内容' }));
    await act(async () => {});
    expect(calls[0].url).toBe('/backup/inspect?path=%2Fsrv%2Fcache%2Fserve-files%2Fx%2Fbackup.zip');
    expect((screen.getByRole('textbox', { name: '备份的路径' }) as HTMLInputElement).value).toBe('/srv/cache/serve-files/x/backup.zip');
    const held = screen.getByLabelText('备份的内容');
    expect(held.textContent).toContain('wxid_demo');
    expect(held.textContent).toContain('数据库1 个 · 1.0 KB');

    const restore = screen.getByRole('button', { name: '恢复' }) as HTMLButtonElement;
    expect(restore.disabled).toBe(true);
    fireEvent.change(screen.getByRole('textbox', { name: '恢复到的文件夹' }), { target: { value: ' /srv/restored ' } });
    fireEvent.click(restore);
    await act(async () => {});
    expect(calls[1]).toEqual({ method: 'POST', url: '/backup/restore', body: { path: '/srv/cache/serve-files/x/backup.zip', target: '/srv/restored' } });
    expect(screen.getByText(/^已开始恢复/)).toBeTruthy();
  });

  it('says why a backup cannot be read', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: false, error: { code: 'runtime_error', message: '不是 WeFlow 备份' } }, 500)));
    render(<BackupPage jobs={NONE} />);
    fireEvent.change(screen.getByRole('textbox', { name: '备份的路径' }), { target: { value: 'C:/x.zip' } });
    fireEvent.click(screen.getByRole('button', { name: '查看' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('不是 WeFlow 备份');
    expect(screen.queryByLabelText('备份的内容')).toBeNull();
  });
});
