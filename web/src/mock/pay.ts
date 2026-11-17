// The sample's WeChat Pay notices: what working on WeFlow cost, the servers and the domain, the coffee and the late
// meals of a release; and what the chats speak of, the anniversary's flowers, the noodles, the calligraphy paper.
import type { PayRecord } from '../api/contract.gen';
import { ago, at } from './clock';

interface Paid {
  at: number;
  merchant: string;
  /** Yuan, two decimals. */
  amount: string;
  method: string;
  item: string;
  refund?: boolean;
}

const paid: Paid[] = [
  { at: ago(0, 6, 5), merchant: '云端主机', amount: '99.00', method: '零钱', item: '基准测试服务器续费（月付）' },
  { at: at(2, '12:05'), merchant: '鲜花店', amount: '199.00', method: '零钱', item: '纪念日的花（明天上午送到）' },
  { at: at(2, '23:48'), merchant: '夜宵外卖', amount: '73.20', method: '零钱', item: '6.0 发版那晚的夜宵' },
  { at: at(3, '10:30'), merchant: '书法用品店', amount: '58.00', method: '零钱', item: '宣纸和毛毡' },
  { at: at(3, '18:52'), merchant: '巷子里的面馆', amount: '46.00', method: '零钱', item: '两碗牛肉面' },
  { at: at(5, '17:20'), merchant: '水果店', amount: '35.80', method: '零钱', item: '回家带的水果' },
  { at: ago(2, 2, 0), merchant: '域名服务', amount: '69.00', method: '储蓄卡(1234)', item: 'WeFlow 文档站域名续费一年' },
  { at: at(6, '14:20'), merchant: '街角咖啡', amount: '56.00', method: '零钱', item: 'WeFlow 线下交流（两杯拿铁）' },
  { at: at(6, '13:35'), merchant: '地铁出行', amount: '4.00', method: '零钱', item: '去线下交流' },
  { at: ago(4, 3, 0), merchant: '图文打印店', amount: '18.50', method: '零钱', item: '打印 WebUI 设计稿' },
  { at: ago(8, 5, 0), merchant: '云端主机', amount: '33.00', method: '零钱', item: '多扣的流量费', refund: true },
  { at: ago(9, 4, 30), merchant: '数码配件店', amount: '129.00', method: '储蓄卡(1234)', item: 'USB-C 扩展坞（macOS 测试机用）' },
  { at: ago(12, 2, 0), merchant: '街角咖啡', amount: '28.00', method: '零钱', item: '改导出格式的下午' },
  { at: ago(20, 6, 0), merchant: '技术书店', amount: '89.00', method: '零钱', item: 'Rust 编程书' },
  { at: ago(30, 6, 5), merchant: '云端主机', amount: '99.00', method: '零钱', item: '基准测试服务器续费（月付）' },
  { at: ago(40, 3, 0), merchant: '云端主机', amount: '46.00', method: '零钱', item: '基准测试流量包' },
];

/** The notices of `biz pay-records`, newest first. */
export const payRecords: PayRecord[] = paid.map((p, i) => ({
  title: p.refund ? '退款到账通知' : '微信支付凭证',
  description: (p.refund
    ? ['退款金额¥' + p.amount, '退款方式' + p.method, '退款原因' + p.item, '商户名称' + p.merchant, '当前状态已到账']
    : ['付款金额¥' + p.amount, '支付方式' + p.method, '商品详情' + p.item, '收款方' + p.merchant, '交易状态支付成功，对方已收款']).join('\n'),
  merchantName: p.merchant, merchantIcon: '', timestamp: p.at, localId: 900 - i, createTime: p.at,
})).sort((a, b) => b.timestamp - a.timestamp);
