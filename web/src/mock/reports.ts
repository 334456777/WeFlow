// The sample's reports: the year of WeFlow 6.0 as an annual report, and a dual report with each friend.
import type { AnnualReport, DualMessage, DualReport } from '../api/contract.gen';
import { ago, dayOf } from './clock';
import { threads } from './chats';
import { ME, nameOf } from './people';
import { timeline } from './moments';

/** The years with messages, the current one first. */
export const reportYears = () => {
  const year = new Date().getFullYear();
  return { years: [year, year - 1, year - 2], strategy: 'native' };
};

const date = (year: number, month: number, day: number) => year + '-' + String(month).padStart(2, '0') + '-' + String(day).padStart(2, '0');

/** Busy evenings, quiet mornings, more at the weekend: a week of hours. */
function heatmap(scale: number): number[][] {
  return Array.from({ length: 7 }, (_, weekday) => Array.from({ length: 24 }, (_, hour) => {
    const evening = hour >= 19 ? 3 + (hour - 19) : hour <= 1 ? 4 - hour * 2 : 0;
    const day = hour >= 9 && hour <= 18 ? 1.4 : 0;
    const weekend = weekday >= 5 ? 1.4 : 1;
    return Math.round((evening + day + (hour >= 2 && hour <= 7 ? 0 : 0.4)) * weekend * scale * (1 + ((weekday * 7 + hour) % 5) / 10));
  }));
}

const FRIENDS: [username: string, sent: number, received: number][] = [
  ['wxid_sample_linmu', 1_820, 2_050], ['wxid_sample_chenyu', 1_610, 1_790], ['wxid_sample_zhouzhou', 1_402, 1_398],
  ['wxid_sample_xuzhiyuan', 905, 1_060], ['wxid_sample_hexi', 640, 702],
];

/** The annual report of `year` (0 for all of them). */
export function annualReport(year: number): AnnualReport {
  const all = year === 0;
  const scale = all ? 2.6 : 1;
  const shown = year || new Date().getFullYear();
  const friends = FRIENDS.map(([username, sent, received]) => ({
    username, displayName: nameOf(username), messageCount: Math.round((sent + received) * scale),
    sentCount: Math.round(sent * scale), receivedCount: Math.round(received * scale),
  }));
  const monthly = ['wxid_sample_hexi', 'wxid_sample_chenyu', 'wxid_sample_chenyu', 'wxid_sample_xuzhiyuan', 'wxid_sample_linmu', 'wxid_sample_linmu',
    'wxid_sample_zhouzhou', 'wxid_sample_chenyu', 'wxid_sample_linmu', 'wxid_sample_zhouzhou', 'wxid_sample_linmu', 'wxid_sample_chenyu'];
  const likers = new Map<string, number>();
  for (const post of timeline.filter(p => p.username === ME)) for (const name of post.likes) likers.set(name, (likers.get(name) ?? 0) + 1);
  return {
    year, totalMessages: Math.round(21_486 * scale), totalFriends: Math.round(46 * (all ? 1.8 : 1)),
    coreFriends: friends,
    monthlyTopFriends: monthly.map((username, i) => ({ month: i + 1, displayName: nameOf(username), messageCount: Math.round((240 + ((i * 53) % 7) * 41) * scale) })),
    peakDay: { date: date(shown, 11, 16), messageCount: 612, topFriend: '陈屿', topFriendCount: 188 },
    longestStreak: { friendName: '林沐', days: all ? 83 : 47, startDate: date(shown, 9, 28), endDate: date(shown, 11, 13) },
    activityHeatmap: { data: heatmap(14 * scale) },
    midnightKing: { displayName: '陈屿', count: Math.round(1_204 * scale), percentage: 14.3 },
    mutualFriend: { displayName: '周舟', sentCount: Math.round(1_402 * scale), receivedCount: Math.round(1_398 * scale), ratio: 1.0 },
    socialInitiative: { initiatedChats: Math.round(312 * scale), receivedChats: Math.round(287 * scale), initiativeRate: 52.1, topInitiatedFriend: '陈屿', topInitiatedCount: Math.round(64 * scale) },
    responseSpeed: { avgResponseTime: 284, fastestFriend: '林沐', fastestTime: 26 },
    topPhrases: [
      ['收到', 612], ['WeFlow', 488], ['好', 455], ['导出', 302], ['皮肤', 214], ['发版', 187], ['基准', 166], ['PR', 152], ['Wiki', 131], ['辛苦了', 97],
    ].map(([phrase, count]) => ({ phrase: String(phrase), count: Math.round(Number(count) * scale) })),
    snsStats: {
      totalPosts: timeline.filter(p => p.username === ME).length, typeCounts: { 1: 3 },
      topLikers: [...likers].sort((a, b) => b[1] - a[1]).slice(0, 3).map(([displayName, count]) => ({ username: displayName, displayName, count })),
      topLiked: [{ username: 'wxid_sample_linmu', displayName: '林沐', count: 6 }, { username: 'wxid_sample_chenyu', displayName: '陈屿', count: 4 }],
    },
    lostFriend: { username: 'wxid_sample_tangguo', displayName: '唐果', earlyCount: 820, lateCount: 12, periodDesc: all ? '前两年' : '上半年' },
  };
}

const dual = (content: string, mine: boolean, at: number, friend: string): DualMessage => ({
  content, isSentByMe: mine, senderUsername: mine ? ME : friend, createTime: at * 1000, createTimeStr: dayOf(at), localType: 1,
});

/** The dual report with `friend` for `year` (0 for all of them): how it began, then what the sample's chat with them holds. */
export function dualReport(friend: string, year: number): DualReport {
  const messages = threads.get(friend) ?? [];
  const firstAt = ago(3 * 365 + 40, 3);
  const name = nameOf(friend);
  const scale = year === 0 ? 2.6 : 1;
  const yearStart = ago(Math.max(1, new Date().getMonth() * 30 + new Date().getDate()), 10);
  const texts = messages.filter(m => m.localType === 1);
  return {
    year, selfName: nameOf(ME), friendUsername: friend, friendName: name,
    firstChat: dual('你好，我是' + name + '，在 issue 里看到 WeFlow，想帮忙', false, firstAt, friend),
    firstChatMessages: [
      dual('你好，我是' + name + '，在 issue 里看到 WeFlow，想帮忙', false, firstAt, friend),
      dual('太好了！先从 Wiki 的「开发指南」看起', true, firstAt + 240, friend),
      dual('好，我先把项目跑起来', false, firstAt + 600, friend),
    ],
    yearFirstChat: {
      createTime: yearStart * 1000, createTimeStr: dayOf(yearStart), content: '新年快乐！今年把 6.0 发出来', isSentByMe: true, friendName: name, localType: 1,
      firstThreeMessages: [dual('新年快乐！今年把 6.0 发出来', true, yearStart, friend), dual('一定 💪', false, yearStart + 120, friend)],
    },
    stats: {
      totalMessages: Math.round(3_870 * scale), totalWords: Math.round(52_400 * scale), imageCount: Math.round(312 * scale),
      voiceCount: Math.round(86 * scale), emojiCount: Math.round(204 * scale),
    },
    topPhrases: [['收到', 96], ['好', 88], ['WeFlow', 61], ['PR', 40], ['截图', 33]].map(([phrase, count]) => ({ phrase: String(phrase), count: Math.round(Number(count) * scale) })),
    myExclusivePhrases: [{ phrase: '已经在改了', count: 21 }, { phrase: '记得跑一下', count: 14 }],
    friendExclusivePhrases: texts.slice(0, 2).map(m => ({ phrase: m.parsedContent.slice(0, 8), count: 9 })),
    heatmap: heatmap(2 * scale),
    initiative: { initiated: Math.round(118 * scale), received: Math.round(131 * scale) },
    response: { avg: 210, fastest: 8, slowest: 5_400, count: Math.round(1_204 * scale) },
    monthly: Object.fromEntries(Array.from({ length: 12 }, (_, i) => {
      const at = new Date(new Date().getFullYear(), new Date().getMonth() - 11 + i, 1);
      return [at.getFullYear() + '-' + String(at.getMonth() + 1).padStart(2, '0'), Math.round((180 + ((i * 37) % 9) * 40) * scale)];
    })),
    streak: { days: 21, startDate: dayOf(ago(40)), endDate: dayOf(ago(20)) },
  };
}
