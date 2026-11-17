// The sample's reports: the year of WeFlow 6.0 as an annual report, where the one the chat calls first is first (and
// still talking after midnight), and a dual report with each friend, its own story for the closest of them.
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

const LU = 'wxid_sample_xiaolu';

/** This year's most talked to: the one the chat calls first, the thirty thousand messages of all years with them. */
const FRIENDS: [username: string, sent: number, received: number][] = [
  [LU, 6_020, 6_380], ['wxid_sample_linmu', 1_820, 2_050], ['wxid_sample_chenyu', 1_610, 1_790], ['wxid_sample_mom', 1_120, 1_460],
  ['wxid_sample_zhouzhou', 1_402, 1_398],
];

/** The annual report of `year` (0 for all of them). */
export function annualReport(year: number): AnnualReport {
  const all = year === 0;
  const scale = all ? 2.6 : 1;
  const shown = year || new Date().getFullYear();
  const friends = FRIENDS.map(([username, sent, received]) => ({
    username, displayName: nameOf(username), messageCount: Math.round((sent + received) * scale),
    sentCount: Math.round(sent * scale), receivedCount: Math.round(received * scale),
  })).sort((a, b) => b.messageCount - a.messageCount);
  const monthly = [LU, LU, 'wxid_sample_chenyu', LU, 'wxid_sample_linmu', LU, 'wxid_sample_mom', LU, 'wxid_sample_linmu', LU, 'wxid_sample_chenyu', LU];
  const likers = new Map<string, number>();
  for (const post of timeline.filter(p => p.username === ME)) for (const name of post.likes) likers.set(name, (likers.get(name) ?? 0) + 1);
  return {
    year, totalMessages: Math.round(31_486 * scale), totalFriends: Math.round(46 * (all ? 1.8 : 1)),
    coreFriends: friends,
    monthlyTopFriends: monthly.map((username, i) => ({ month: i + 1, displayName: nameOf(username), messageCount: Math.round((240 + ((i * 53) % 7) * 41 + (username === LU ? 520 : 0)) * scale) })),
    peakDay: { date: date(shown, 11, 16), messageCount: 612, topFriend: '陈屿', topFriendCount: 188 },
    longestStreak: { friendName: '小鹿', days: all ? 618 : 214, startDate: date(shown, 4, 18), endDate: date(shown, 11, 17) },
    activityHeatmap: { data: heatmap(14 * scale) },
    // who is still talking after midnight: the one telling to go to sleep
    midnightKing: { displayName: '小鹿', count: Math.round(1_904 * scale), percentage: 31.2 },
    mutualFriend: { displayName: '周舟', sentCount: Math.round(1_402 * scale), receivedCount: Math.round(1_398 * scale), ratio: 1.0 },
    socialInitiative: { initiatedChats: Math.round(312 * scale), receivedChats: Math.round(287 * scale), initiativeRate: 52.1, topInitiatedFriend: '小鹿', topInitiatedCount: Math.round(96 * scale) },
    responseSpeed: { avgResponseTime: 284, fastestFriend: '小鹿', fastestTime: 12 },
    topPhrases: [
      ['好', 1_204], ['收到', 612], ['早点睡', 520], ['WeFlow', 488], ['哈哈哈', 455], ['晚安', 366], ['导出', 302], ['皮肤', 214], ['发版', 187], ['回家吃饭', 140],
    ].map(([phrase, count]) => ({ phrase: String(phrase), count: Math.round(Number(count) * scale) })),
    snsStats: {
      totalPosts: timeline.filter(p => p.username === ME).length, typeCounts: { 1: 3 },
      topLikers: [...likers].sort((a, b) => b[1] - a[1]).slice(0, 3).map(([displayName, count]) => ({ username: displayName, displayName, count })),
      topLiked: [{ username: LU, displayName: '小鹿', count: 9 }, { username: 'wxid_sample_linmu', displayName: '林沐', count: 6 }, { username: 'wxid_sample_mom', displayName: '妈妈', count: 5 }],
    },
    lostFriend: { username: 'wxid_sample_tangguo', displayName: '唐果', earlyCount: 820, lateCount: 12, periodDesc: all ? '前两年' : '上半年' },
  };
}

const dual = (content: string, mine: boolean, at: number, friend: string): DualMessage => ({
  content, isSentByMe: mine, senderUsername: mine ? ME : friend, createTime: at * 1000, createTimeStr: dayOf(at), localType: 1,
});

/** How it began with someone, what the new year started with, and this year's messages, where it is not WeFlow. */
interface Story { first: [text: string, mine: boolean][]; newYear: [text: string, mine: boolean][]; messages: number; phrases: [string, number][]; mine: string[] }

const STORIES: Record<string, Story> = {
  // the thirty thousand messages of all years the chat speaks of
  [LU]: {
    first: [['你好，我是读书会上坐你旁边的那个', false], ['我记得，你借了我一支笔', true], ['那支笔还没还你 😅', false]],
    newYear: [['新年快乐，今年也请多指教', true], ['新年快乐！今年少熬夜', false]],
    messages: 12_400, phrases: [['早点睡', 312], ['好', 290], ['晚安', 266], ['哈哈哈', 201], ['想你', 88]], mine: ['马上就睡', '你决定'],
  },
  'wxid_sample_mom': {
    first: [['孩子，这是妈妈的微信', false], ['收到，已经加上了', true], ['以后有事就在这儿说', false]],
    newYear: [['新年快乐！今年早点回家过年', false], ['一定！', true]],
    messages: 2_580, phrases: [['吃饭', 166], ['回家', 120], ['好', 98], ['注意身体', 61], ['知道啦', 55]], mine: ['知道啦', '回！'],
  },
  'wxid_sample_ajie': {
    first: [['404 的兄弟们拉个群？', false], ['拉！', true], ['我来建', false]],
    newYear: [['新年快乐，今年乒乓球赢你', false], ['做梦 😏', true]],
    messages: 1_190, phrases: [['单挑', 41], ['奶茶', 33], ['哈哈哈', 30], ['老地方', 21], ['周三', 18]], mine: ['对手发挥得好', '删不了'],
  },
};

/** The dual report with `friend` for `year` (0 for all of them): how it began, then what the sample's chat with them holds. */
export function dualReport(friend: string, year: number): DualReport {
  const messages = threads.get(friend) ?? [];
  const firstAt = ago(3 * 365 + 40, 3);
  const name = nameOf(friend);
  const scale = year === 0 ? 2.6 : 1;
  const yearStart = ago(Math.max(1, new Date().getMonth() * 30 + new Date().getDate()), 10);
  const texts = messages.filter(m => m.localType === 1);
  const story: Story = STORIES[friend] ?? {
    first: [['你好，我是' + name + '，在 issue 里看到 WeFlow，想帮忙', false], ['太好了！先从 Wiki 的「开发指南」看起', true], ['好，我先把项目跑起来', false]],
    newYear: [['新年快乐！今年把 6.0 发出来', true], ['一定 💪', false]],
    messages: 3_870, phrases: [['收到', 96], ['好', 88], ['WeFlow', 61], ['PR', 40], ['截图', 33]], mine: ['已经在改了', '记得跑一下'],
  };
  const firsts = story.first.map(([text, mine], i) => dual(text, mine, firstAt + i * 240, friend));
  const yearFirsts = story.newYear.map(([text, mine], i) => dual(text, mine, yearStart + i * 120, friend));
  const total = story.messages;
  return {
    year, selfName: nameOf(ME), friendUsername: friend, friendName: name,
    firstChat: firsts[0],
    firstChatMessages: firsts,
    yearFirstChat: {
      createTime: yearStart * 1000, createTimeStr: dayOf(yearStart), content: story.newYear[0][0], isSentByMe: story.newYear[0][1], friendName: name, localType: 1,
      firstThreeMessages: yearFirsts,
    },
    stats: {
      totalMessages: Math.round(total * scale), totalWords: Math.round(total * 13.5 * scale), imageCount: Math.round(total * 0.08 * scale),
      voiceCount: Math.round(total * 0.022 * scale), emojiCount: Math.round(total * 0.053 * scale),
    },
    topPhrases: story.phrases.map(([phrase, count]) => ({ phrase, count: Math.round(count * scale) })),
    myExclusivePhrases: story.mine.map((phrase, i) => ({ phrase, count: 21 - i * 7 })),
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
