import { act, cleanup, renderHook } from '@testing-library/react';
import { strFromU8, unzipSync } from 'fflate';
import { afterEach, describe, expect, it, vi } from '#test';
import { emitLive, LIVE_COMMAND_MS } from '../api/live';
import type { Payment } from '../api/pay';
import { exportPayments } from './payExport';
import { ALL_PAYMENTS, matches, merchantsOf, totalsOf, usePayments, withinDays } from './usePayments';

const at = (day: number) => Math.floor(new Date(2026, 9, day, 9).getTime() / 1000);
const pay = (id: string, merchant: string, amount: number | null, day: number, refund = false): Payment =>
  ({ id, time: at(day), title: '', merchant, amount, refund, method: '零钱', status: '', lines: [{ label: '付款金额', value: '¥' + amount }] });

const payments = [pay('a', '咖啡', 2800, 12), pay('b', '咖啡', 500, 11, true), pay('c', '出行', 1250, 2), pay('d', '出行', null, 1)];

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('payments', () => {
  it('reads the notices once per server or account, and again after a live message may have brought one', async () => {
    vi.useFakeTimers();
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      const records = [{ localId: urls.length, createTime: 1_760_000_000 + urls.length, title: '微信支付凭证', description: '付款金额¥28.00', merchantName: '咖啡', merchantIcon: '', timestamp: 0 }];
      return new Response(JSON.stringify({ success: true, data: { records, hasMore: false } }));
    }));
    const { result, rerender } = renderHook(({ enabled, scope }) => usePayments(enabled, scope), { initialProps: { enabled: true, scope: 1 } });
    await act(async () => {});
    expect(result.current[0].status).toBe('ready');
    expect(urls).toEqual(['/api/biz/pay-records?limit=500&offset=0']);

    // the page left and opened again
    rerender({ enabled: false, scope: 1 });
    rerender({ enabled: true, scope: 1 });
    await act(async () => {});
    expect(urls).toHaveLength(1);

    // a message of another conversation brings no notice
    act(() => emitLive({ type: 'message.new', sessionId: 'wxid_a' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_COMMAND_MS); });
    expect(urls).toHaveLength(1);

    // one of the WeChat Pay account while the page is left: read once it opens again
    rerender({ enabled: false, scope: 1 });
    act(() => emitLive({ type: 'message.new', sessionId: 'gh_3dfda90e39d6' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_COMMAND_MS); });
    expect(urls).toHaveLength(1);
    rerender({ enabled: true, scope: 1 });
    await act(async () => {});
    expect(urls).toHaveLength(2);

    // a service notice while the page is open: read at once, the notices read before shown meanwhile
    act(() => emitLive({ type: 'message.new', sessionId: 'notifymessage' }));
    await act(async () => { await vi.advanceTimersByTimeAsync(LIVE_COMMAND_MS); });
    expect(urls).toHaveLength(3);
    expect(result.current[0].status).toBe('ready');

    // another server or account
    rerender({ enabled: true, scope: 2 });
    await act(async () => {});
    expect(urls).toHaveLength(4);
  });

  it('narrows them by days and payee', () => {
    expect(withinDays(payments, { from: '2026-10-02', to: '2026-10-11' }).map(p => p.id)).toEqual(['b', 'c']);
    expect(withinDays(payments, { from: '2026-10-11', to: '' }).map(p => p.id)).toEqual(['a', 'b']);
    expect(matches(payments, { ...ALL_PAYMENTS, merchant: '出行' }).map(p => p.id)).toEqual(['c', 'd']);
  });

  it('adds them up per payee and in all, refunds apart', () => {
    expect(merchantsOf(payments).map(m => [m.name, m.count, m.spent])).toEqual([['咖啡', 2, 2300], ['出行', 2, 1250]]);
    expect(totalsOf(payments)).toEqual({ count: 4, paid: 4050, refunded: 500, unknown: 1 });
  });

  it('exports the ones chosen as Excel, JSON or HTML', async () => {
    const saved: Blob[] = [];
    vi.spyOn(URL, 'createObjectURL').mockImplementation(blob => { saved.push(blob as Blob); return 'blob:x'; });
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    const filter = { merchant: '咖啡', from: '2026-10-01', to: '' };
    const chosen = matches(payments, filter);

    expect((await exportPayments(chosen, filter, 'Excel')).fileName).toBe('微信支付_咖啡_2026-10-01_最近.xlsx');
    const sheet = strFromU8(unzipSync(new Uint8Array(await saved[0].arrayBuffer()))['xl/worksheets/sheet1.xml']);
    expect(sheet).toContain('<t xml:space="preserve">退款</t>');
    expect(sheet).toContain('<t xml:space="preserve">28.00</t>');

    await exportPayments(chosen, filter, 'JSON');
    const body = JSON.parse(await saved[1].text());
    expect(body).toMatchObject({ merchant: '咖啡', count: 2, paidFen: 2800, refundedFen: 500 });

    expect((await exportPayments(chosen, filter, 'HTML')).count).toBe(2);
    expect(await saved[2].text()).toContain('2 笔 · 支出 ¥28.00 · 退款 ¥5.00');
    await expect(exportPayments([], filter, 'HTML')).rejects.toThrow('筛选范围内没有支付记录。');
  });
});
