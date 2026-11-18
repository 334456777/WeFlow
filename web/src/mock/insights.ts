// The sample's AI insights: what the insight engine noticed in the chats, about WeFlow and about the account's own life,
// with how each was written. Read, cleared and written anew in the page while it is open (#896).
import type { InsightCleared, InsightContact, InsightMarked, InsightRecord, InsightReply, InsightSummary } from '../api/contract.gen';
import { threads } from './chats';
import { ago, at, dayOf, NOW } from './clock';
import { ME, nameOf } from './people';

interface Written { id: string; session: string; at: number; reason: 'activity' | 'silence' | 'test'; text: string; read: boolean }

const written: Written[] = [
  { id: 'sample-insight-9', session: 'wxid_sample_xiaolu', at: at(1, '23:45'), reason: 'activity', read: false,
    text: '小鹿第二次说「你上次也这么说」了。纪念日的假已经请好、花也订了，这次的「马上就睡」可以先做到。' },
  { id: 'sample-insight-8', session: 'wxid_sample_mom', at: at(1, '20:05'), reason: 'activity', read: false,
    text: '妈妈问周末回不回家吃饭，还说要带上小鹿，已经答应了。可以提前问问家里缺什么，顺路带上；外婆的语音也可以再拷一份给家里。' },
  { id: 'sample-insight-7', session: 'wxid_sample_ajie', at: ago(2, 4, 30), reason: 'silence', read: true,
    text: '阿杰约了下周乒乓球单挑，输的人请奶茶。毕业十年的聚会也定在下周六，可以把 404 寝室的老照片找出来带上。' },
  { id: 'sample-insight-6', session: 'wxid_sample_linmu', at: ago(0, 0, 40), reason: 'activity', read: false,
    text: '林沐刚发来浅色版的截图，准备提 PR。可以提醒跑一下 bun run shots 前后对比，再一起看看控制台卡片的间距。' },
  { id: 'sample-insight-5', session: 'wxid_sample_zhouzhou', at: ago(0, 1, 30), reason: 'activity', read: false,
    text: '周舟把导出的 HTML 发了过来，还转了一笔咖啡钱表示感谢。这次导出的是 ChatLab 和 HTML，可以问问 WeClone 格式用得顺不顺手。' },
  { id: 'sample-insight-4', session: 'wxid_sample_chenyu', at: ago(0, 4, 50), reason: 'activity', read: true,
    text: '陈屿提到基准测试的机器今晚维护，下周想试 PGO。可以把 WebUI 常用的几个读取加进 pgo-build.sh 的训练清单。' },
  { id: 'sample-insight-3', session: 'wxid_sample_gunan', at: ago(1, 3, 0), reason: 'activity', read: true,
    text: '顾南已经把十年的聊天记录导出来了，还帮家人也导了一份。提醒过数据库备份，之后可以问问备份是否顺利。' },
  { id: 'sample-insight-2', session: 'wxid_sample_songxiaoman', at: ago(2, 5, 0), reason: 'silence', read: true,
    text: '宋小满五天前在 Linux 上跑起了 weflow serve，之后没有再聊。可以请宋小满帮忙在 Linux 上试一下新版的网页。' },
  { id: 'sample-insight-1', session: 'wxid_sample_hexi', at: ago(4, 2, 0), reason: 'test', read: true,
    text: '何夕已经在 macOS 上取到了图片密钥。可以请何夕把步骤写进 Wiki 的「密钥获取」，给用户交流群里遇到同样问题的人参考。' },
];

const summaryOf = (w: Written): InsightSummary => ({
  id: w.id, createdAt: w.at * 1000, sessionId: w.session, displayName: nameOf(w.session), triggerReason: w.reason, insight: w.text, read: w.read,
});

/** A page of `insight records`: of everyone or one conversation, found by words, newest first. */
export function insightRecords(keyword: string, session: string, offset: number, limit: number) {
  const matching = written.filter(w => (!session || w.session === session) && (!keyword || w.text.includes(keyword) || nameOf(w.session).includes(keyword)))
    .sort((a, b) => b.at - a.at);
  const counts = new Map<string, number>();
  for (const w of written) counts.set(w.session, (counts.get(w.session) ?? 0) + 1);
  const contacts: InsightContact[] = [...counts].map(([sessionId, count]) => ({ sessionId, displayName: nameOf(sessionId), count }));
  const today = dayOf(NOW);
  return {
    records: matching.slice(offset, offset + limit).map(summaryOf), total: matching.length,
    todayCount: written.filter(w => dayOf(w.at) === today).length, unreadCount: written.filter(w => !w.read).length, contacts,
  };
}

/** One record with how it was written (`insight get`). */
export function insightRecord(id: string): { success: boolean; record?: InsightRecord; error?: string } {
  const w = written.find(r => r.id === id);
  if (!w) return { success: false, error: 'not found' };
  const name = nameOf(w.session);
  return {
    success: true,
    record: {
      ...summaryOf(w), accountScope: ME,
      log: {
        endpoint: 'https://api.example.com/v1/chat/completions', model: 'sample-model', maxTokens: 512, temperature: 0.7,
        triggerReason: w.reason, allowContext: true, contextCount: 20,
        systemPrompt: '你是用户的社交助理，根据最近的聊天，用一两句话提醒用户可以做什么。只写具体的事，不编造聊天里没有的内容。',
        userPrompt: '和「' + name + '」最近 20 条消息（示例数据，不是真实的聊天）……',
        rawOutput: w.text, finalInsight: w.text, durationMs: 1_840, createdAt: w.at * 1000,
      },
    },
  };
}

/** `insight mark-read <id>`. */
export function markRead(id: string): InsightMarked {
  const w = written.find(r => r.id === id);
  if (!w) return { success: false, error: 'not found' };
  w.read = true;
  return { success: true };
}

/** `insight clear [session]`: every record, or one conversation's. */
export function clearInsights(session: string): InsightCleared {
  const before = written.length;
  for (let i = written.length - 1; i >= 0; i--) if (!session || written[i].session === session) written.splice(i, 1);
  return { success: true, removed: before - written.length };
}

let made = 0;

/** The private chats, the one talked in last first. */
const privateChats = () => [...threads].filter(([id]) => id.startsWith('wxid_') && id !== ME)
  .sort(([, a], [, b]) => b[b.length - 1].createTime - a[a.length - 1].createTime);

/**
 * `insight trigger [session]`: one insight now, of that conversation or of the private chat talked in last, from the
 * last thing said in it with something to it: the other side's first, a reply of a word or two passed over.
 */
export function triggerInsight(session: string): InsightReply {
  const id = session || privateChats()[0]?.[0];
  const texts = [...(id ? threads.get(id) ?? [] : [])].reverse().filter(m => m.localType === 1);
  const said = texts.find(m => m.senderUsername !== ME && m.parsedContent.length >= 8) ?? texts.find(m => m.parsedContent.length >= 8) ?? texts[0];
  if (!id || !said) return { success: false, message: '这个会话最近没有可以总结的文字消息。' };
  const name = nameOf(id);
  const line = said.parsedContent.length > 24 ? said.parsedContent.slice(0, 24) + '…' : said.parsedContent;
  made += 1;
  const w: Written = {
    id: 'sample-insight-new-' + made, session: id, at: Math.floor(Date.now() / 1000), reason: 'test', read: false,
    text: `和${name}最近聊到「${line}」。可以接着这件事回一句，问问后来怎么样了。`,
  };
  written.push(w);
  return { success: true, message: '已生成一条见解。', record: summaryOf(w) };
}
