// Sample records the screens show while no `weflow serve` is connected (#130).
import type { ChatSession } from '../api/chat';
import type { Category, Conversation, Message } from '../types';
import photo from './street.svg';

export const CURRENT_DATE = '2026-10-12';
export const DEFAULT_FROM = '2026-10-01';
export const categoryLabels: Record<Category, string> = {
  chat: '聊天', moments: '朋友圈', payments: '微信支付',
};

function text(id: string, sender: string, content: string, date: string, time: string, outgoing = false): Message {
  return { id, sender, text: content, date, time, kind: 'text', direction: outgoing ? 'outgoing' : 'incoming' };
}

export const conversations: Conversation[] = [
  {
    id: 'lin', category: 'chat', name: '林沐',
    preview: '照片整理好了，发给你看看。', stamp: '14:12', description: '个人聊天 · 示例记录',
    messages: [
      text('lin-1', '林沐', '照片整理好了，发给你看看。', CURRENT_DATE, '14:12'),
      { id: 'lin-2', sender: '林沐', date: CURRENT_DATE, time: '14:12', kind: 'image', direction: 'incoming', text: '街景照片', asset: photo, fileName: '街景.svg' },
      text('lin-3', '我', '这张光线很好。', CURRENT_DATE, '14:18', true),
      text('lin-4', '林沐', '原图也保留了，导出时可以一起保存。', CURRENT_DATE, '14:20'),
    ],
  },
  {
    id: 'chen', category: 'chat', name: '陈屿',
    preview: '好的，周一见。', stamp: '昨天', description: '个人聊天 · 示例记录',
    messages: [
      text('chen-1', '陈屿', '明天一起整理一下旅行照片？', '2026-10-11', '18:32'),
      text('chen-2', '我', '可以，我把文件也带过来。', '2026-10-11', '18:36', true),
      text('chen-3', '陈屿', '好的，周一见。', '2026-10-11', '18:38'),
    ],
  },
  {
    id: 'design@chatroom', category: 'chat', name: '设计协作群',
    preview: '李婷：这版看起来不错 👍', stamp: '10/08', description: '群聊 · 4 位成员 · 示例记录',
    messages: [
      text('design-1', '李婷', '把浏览和操作分开，读消息的时候更清楚。', '2026-10-08', '10:05'),
      text('design-2', '我', '下面用按钮组切换导出和搜索。', '2026-10-08', '10:08', true),
      text('design-3', '李婷', '这版看起来不错 👍', '2026-10-08', '10:12'),
    ],
  },
  {
    id: 'filehelper', category: 'chat', name: '文件传输助手',
    preview: '旅行计划.pdf', stamp: '10/06', description: '文件记录 · 示例内容',
    messages: [
      text('file-1', '我', '这次旅行的计划先保存在这里。', '2026-10-06', '09:30', true),
      { id: 'file-2', sender: '我', kind: 'file', direction: 'outgoing', text: '旅行计划.pdf', fileName: '旅行计划.pdf', date: '2026-10-06', time: '09:31' },
    ],
  },
  {
    id: 'moment-lin', category: 'moments', name: '林沐',
    preview: '傍晚散步，收集一些光线。', stamp: '今天', description: '朋友圈 · 示例记录',
    messages: [
      text('moment-1', '林沐', '傍晚散步，收集一些光线。', CURRENT_DATE, '12:18'),
      { id: 'moment-2', sender: '林沐', kind: 'image', direction: 'incoming', text: '今天的街景', asset: photo, fileName: '街景.svg', date: CURRENT_DATE, time: '12:18' },
    ],
  },
  {
    id: 'moment-chen', category: 'moments', name: '陈屿',
    preview: '周末，慢慢走。', stamp: '昨天', description: '朋友圈 · 示例记录',
    messages: [text('moment-3', '陈屿', '周末，慢慢走。', '2026-10-11', '16:30')],
  },
  {
    id: 'gh_design', category: 'chat', name: '设计观察',
    preview: '让工具回到清楚、简单的使用方式', stamp: '10/10', description: '公众号 · 示例文章',
    messages: [
      text('article-1', '设计观察', '让工具回到清楚、简单的使用方式', '2026-10-10', '09:00'),
      text('article-2', '设计观察', '将阅读区和操作区清晰分开，减少重复入口，让用户专注当前任务。', '2026-10-10', '09:00'),
    ],
  },
  {
    id: 'gh_city', category: 'chat', name: '城市手记',
    preview: '一条小街的午后', stamp: '10/07', description: '公众号 · 示例文章',
    messages: [
      text('article-3', '城市手记', '一条小街的午后', '2026-10-07', '11:45'),
      { id: 'article-4', sender: '城市手记', kind: 'image', direction: 'incoming', text: '午后街景', asset: photo, date: '2026-10-07', time: '11:45', fileName: '午后街景.svg' },
    ],
  },
  {
    id: 'payment-coffee', category: 'payments', name: '街角咖啡',
    preview: '付款成功 · ¥28.00', stamp: '今天', description: '微信支付 · 示例账单',
    messages: [text('pay-1', '微信支付', '付款成功\n商户：街角咖啡\n金额：¥28.00\n方式：余额\n交易时间：2026-10-12 08:45', CURRENT_DATE, '08:45')],
  },
  {
    id: 'payment-travel', category: 'payments', name: '出行服务',
    preview: '付款成功 · ¥12.00', stamp: '昨天', description: '微信支付 · 示例账单',
    messages: [text('pay-2', '微信支付', '付款成功\n商户：出行服务\n金额：¥12.00\n方式：余额\n交易时间：2026-10-11 17:40', '2026-10-11', '17:40')],
  },
];

/** The sample chats as `chat sessions` rows, for the conversation list while no server is connected. */
export const sampleSessions: ChatSession[] = conversations.filter(c => c.category === 'chat').map(c => {
  const last = c.messages[c.messages.length - 1];
  const time = Math.floor(new Date(last.date + 'T' + last.time + ':00').getTime() / 1000);
  const group = c.id.endsWith('@chatroom');
  return {
    username: c.id, displayName: c.name, summary: last.kind === 'text' ? last.text : last.kind === 'image' ? '[图片]' : '[文件] ' + (last.fileName ?? ''),
    unreadCount: 0, sortTimestamp: time, lastTimestamp: time, lastMsgType: 1,
    ...(group && last.direction === 'incoming' ? { lastSenderDisplayName: last.sender } : {}),
  };
}).sort((a, b) => b.sortTimestamp - a.sortTimestamp);

export function formatDay(date: string): string {
  const value = new Date(date + 'T12:00:00');
  return String(value.getFullYear()) + '年' + String(value.getMonth() + 1) + '月' + String(value.getDate()) + '日  ' + ['周日', '周一', '周二', '周三', '周四', '周五', '周六'][value.getDay()];
}
