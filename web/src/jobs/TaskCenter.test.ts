import { describe, expect, it } from '#test';
import { describeCommand, type Job } from '../api/jobs';
import { formatSize, jobDetail, jobsSummary } from './TaskCenter';

const job = (status: Job['status'], more: Partial<Job> = {}): Job =>
  ({ id: 'j', status, command: ['weflow', '--json', '--progress', 'export', 'messages', '--format', 'html', 'wxid_a'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null, ...more });

describe('task centre', () => {
  it('names a job by its command, and by the conversation it exports when known', () => {
    expect(describeCommand(job('queued').command)).toBe('导出聊天记录');
    expect(describeCommand(job('queued').command, new Map([['wxid_a', '林沐']]))).toBe('导出聊天记录 · 林沐');
    expect(describeCommand(['weflow', '--json', 'backup', 'create'])).toBe('备份数据库');
    expect(describeCommand(['weflow', '--json', 'cache', 'clear'])).toBe('cache clear');
  });

  it('says how far a job is, why it failed, or what it wrote', () => {
    expect(jobDetail(job('running', { progress: { message: '正在导出消息', current: 5, total: 10, percent: 50 } }))).toBe('正在导出消息 · 5 / 10 · 50%');
    expect(jobDetail(job('running'))).toBe('正在运行');
    expect(jobDetail(job('failed', { result: { success: false, error: { message: 'no such session' } } }))).toBe('no such session');
    expect(jobDetail(job('failed', { httpStatus: 500 }))).toBe('命令没有完成（HTTP 500）');
    expect(jobDetail(job('succeeded', { file: { name: 'a.html', size: 3350170 } }))).toBe('a.html · 3.2 MB');
    expect(formatSize(512)).toBe('512 B');
  });

  it('sums up what runs and waits', () => {
    expect(jobsSummary({ jobs: [job('running'), job('queued'), job('queued')], labels: {} })).toBe('1 个运行中 · 2 个排队');
    expect(jobsSummary({ jobs: [job('succeeded')], labels: {} })).toBe('1 个任务');
    expect(jobsSummary({ jobs: [], labels: {} })).toBe('没有任务');
  });
});
