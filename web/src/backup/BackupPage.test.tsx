import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { backupParams, backupParts } from '../api/backup';
import type { Job } from '../api/jobs';
import type { JobsState } from '../jobs/useJobs';
import type { Shell } from '../shell/page';
import { SampleContext } from '../shell/sample';
import { BackupPage, backupPage } from './BackupPage';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const job = (over: Partial<Job>): Job => ({
  id: 'j1', status: 'succeeded', command: ['weflow', '--json', '--progress', 'backup', 'create'], createdAt: Date.UTC(2026, 9, 19, 12),
  startedAt: null, finishedAt: null, exitCode: 0, httpStatus: 200, progress: null, file: null, ...over,
});
const NONE: JobsState = { jobs: [], labels: {} };

describe('backup', () => {
  it('groups what a backup holds the way `backup create` collects it, and adds only what is ticked', () => {
    expect(backupParts([
      { path: 'db_storage/message/message_0.db', sha256: '', size: 300 },
      { path: 'db_storage\\contact\\contact.db', sha256: '', size: 100 },
      { path: 'msg/attach/0a1b/2026-10/Img/a.dat', sha256: '', size: 20 },
      { path: 'msg/video/2026-10/b.mp4', sha256: '', size: 70 },
      { path: 'msg/file/2026-10/c.pdf', sha256: '', size: 9 },
      { path: 'emojis/c.gif', sha256: '', size: 5 },
      // a backup of an older WeFlow
      { path: 'FileStorage/Image/2026-10/b.dat', sha256: '', size: 30 },
      { path: 'FileStorage/Audio/2026-10/d.amr', sha256: '', size: 2 },
      { path: 'manifest.extra', sha256: '', size: 1 },
    ])).toEqual([
      { label: '数据库', files: 2, bytes: 400 },
      { label: '图片', files: 2, bytes: 50 },
      { label: '视频', files: 1, bytes: 70 },
      { label: '文件', files: 1, bytes: 9 },
      { label: '语音', files: 1, bytes: 2 },
      { label: '表情', files: 1, bytes: 5 },
      { label: '其他', files: 1, bytes: 1 },
    ]);
    expect(JSON.parse(JSON.stringify(backupParams({ images: false, videos: false, files: false, emojis: false })))).toEqual({});
    expect(JSON.parse(JSON.stringify(backupParams({ images: true, videos: false, files: true, emojis: false })))).toEqual({ images: true, files: true });
  });

  it('makes a backup of the databases as a job that keeps the file, with the parts ticked', async () => {
    const calls: { url: string; body: unknown; prefer: string | undefined }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ url, body: JSON.parse(String(init?.body)), prefer: (init?.headers as Record<string, string>)?.Prefer });
      return json({ success: true, data: job({ status: 'queued' }) }, 202);
    }));
    render(<BackupPage jobs={NONE} />);
    fireEvent.click(screen.getByRole('checkbox', { name: '视频' }));
    fireEvent.click(screen.getByRole('button', { name: '创建备份' }));
    await act(async () => {});
    expect(calls).toEqual([{ url: '/backup/create?return=file', body: { videos: true }, prefer: 'respond-async' }]);
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

  it('says what a backup lacks: files it could not read, databases WeChat kept writing to (#817)', async () => {
    const skipped = ['a', 'b', 'c', 'd'].map(n => ({ path: `msg/attach/${n}.dat`, reason: 'denied' }));
    const jobs: JobsState = {
      jobs: [job({ file: { name: 'backup.zip', size: 2048 }, result: { success: true, data: { out: '/srv/b.zip', skipped, unsettled: ['db_storage/message/message_0.db'] } } })],
      labels: { j1: '备份数据库' },
    };
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { version: '1', created_at: 1, wxid: 'w', weflow_version: '6', entries: [], skipped: skipped.slice(0, 1) } })));
    render(<BackupPage jobs={jobs} />);
    const row = screen.getByRole('list', { name: '这个标签页创建的备份' }).querySelector('li')!;
    expect([...row.querySelectorAll('.backup-gap')].map(p => p.textContent)).toEqual([
      '4 个文件读不了，没有备份进去：msg/attach/a.dat、msg/attach/b.dat、msg/attach/c.dat等',
      '1 个数据库在复制时一直被微信写入，备份里的数据库和它的 WAL 可能不是同一时刻的：db_storage/message/message_0.db',
    ]);
    fireEvent.click(screen.getByRole('button', { name: '查看内容' }));
    await act(async () => {});
    expect(screen.getByLabelText('备份的内容').querySelector('.backup-gap')?.textContent).toBe('1 个文件读不了，没有备份进去：msg/attach/a.dat');
  });

  it('starts afresh for another server or account: the sample\'s backup is not looked at on the server', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { version: '1', created_at: 1, wxid: 'wxid_sample', weflow_version: '6', entries: [] } })));
    const shell = (sample: boolean, scope: number) => ({ backend: sample ? { state: 'offline' } : { state: 'offline', held: true }, jobs: NONE, scope }) as unknown as Shell;
    const page = (sample: boolean, scope: number) => <SampleContext.Provider value={sample}>{backupPage(shell(sample, scope)).view}</SampleContext.Provider>;
    const { rerender } = render(page(true, 1));
    const path = () => (screen.getByRole('textbox', { name: '备份的路径' }) as HTMLInputElement).value;
    expect(path()).toBe('weflow-backup.zip');
    fireEvent.click(screen.getByRole('button', { name: '查看' }));
    await act(async () => {});
    expect(screen.getByLabelText('备份的内容').textContent).toContain('wxid_sample');
    // a server takes the sample's place: a new scope
    rerender(page(false, 2));
    expect(path()).toBe('');
    expect(screen.queryByLabelText('备份的内容')).toBeNull();
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
