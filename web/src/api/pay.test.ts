import { describe, expect, it } from 'vitest';
import { fenOf, formatYuan, paymentOf, type PayRecord } from './pay';

const record = (title: string, description: string, more: Partial<PayRecord> = {}): PayRecord =>
  ({ localId: 1, createTime: 1760000000, title, description, merchantName: '', merchantIcon: '', timestamp: 0, ...more });

describe('WeChat Pay notices', () => {
  it('reads amounts in fen, with or without the sign, commas and fen', () => {
    expect(fenOf('¥28.00')).toBe(2800);
    expect(fenOf('￥ 1,280.5')).toBe(128050);
    expect(fenOf('12元')).toBe(1200);
    expect(fenOf('零钱')).toBeNull();
    expect(formatYuan(128050)).toBe('1,280.50');
  });

  it('takes the amount, method, state and payee from the labelled lines, colon or not', () => {
    const p = paymentOf(record('微信支付凭证', '付款金额¥28.00\n支付方式：零钱\n交易状态 支付成功\n收款方街角咖啡\n商品详情 拿铁', { merchantName: '微信支付', timestamp: 1759990000 }));
    expect(p).toMatchObject({ amount: 2800, refund: false, method: '零钱', status: '支付成功', merchant: '街角咖啡', time: 1759990000 });
    expect(p.lines.at(-1)).toEqual({ label: '商品详情', value: '拿铁' });
  });

  it('knows a refund, and falls back to the title and the notice’s payee', () => {
    expect(paymentOf(record('退款到账', '退款金额¥5.00\n退款方式零钱'))).toMatchObject({ amount: 500, refund: true, method: '零钱', merchant: '微信支付' });
    expect(paymentOf(record('已支付¥9.90', '', { merchantName: '便利店' }))).toMatchObject({ amount: 990, merchant: '便利店', time: 1760000000 });
    expect(paymentOf(record('账单提醒', '本月账单已出')).amount).toBeNull();
  });
});
