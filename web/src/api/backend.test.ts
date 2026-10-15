import { afterEach, describe, expect, it, vi } from 'vitest';
import { checkBackend, compareVersions, incompatibility } from './backend';

const web = { name: 'weflow-web', version: '0.1.0', apiVersion: '1', minBackendVersion: '5.0.0-rc.4' };

afterEach(() => vi.unstubAllGlobals());

describe('compareVersions', () => {
  it('orders versions as SemVer does', () => {
    const sorted = ['4.2.1', '5.0.0-rc.3', '5.0.0-rc.4', '5.0.0-rc.4-3-gabc1234', '5.0.0-rc.10', '5.0.0', '5.0.1', '5.1.0'];
    for (let i = 1; i < sorted.length; i++) expect(compareVersions(sorted[i - 1], sorted[i]), sorted[i - 1] + ' < ' + sorted[i]).toBeLessThan(0);
    expect(compareVersions('v5.0.0', '5.0.0')).toBe(0);
    expect(compareVersions('5.0.0-alpha', '5.0.0-alpha.1')).toBeLessThan(0);
    expect(compareVersions('5.0.0-1', '5.0.0-alpha')).toBeLessThan(0);
  });

  it('gives NaN for something that is not a version', () => {
    expect(compareVersions('unknown', '5.0.0')).toBeNaN();
  });
});

describe('incompatibility', () => {
  it('accepts the same API on a new enough backend', () => {
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.4' }, web)).toBeNull();
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.4-12-g54c22b9' }, web)).toBeNull();
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.2.0' }, web)).toBeNull();
  });

  it('refuses another API or an older backend', () => {
    expect(incompatibility({ status: 'ok', apiVersion: '2', version: '6.0.0' }, web)).toContain('API');
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.3' }, web)).toContain('5.0.0-rc.4');
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: 'unknown' }, web)).not.toBeNull();
  });
});

describe('checkBackend', () => {
  it('reads /health', async () => {
    const health = { status: 'ok', apiVersion: '1', version: '9.0.0' };
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(health))));
    await expect(checkBackend()).resolves.toEqual({ state: 'ready', health });
  });

  it('is offline without a server or with an error', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));
    await expect(checkBackend()).resolves.toEqual({ state: 'offline' });
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 502 })));
    await expect(checkBackend()).resolves.toEqual({ state: 'offline' });
  });
});
