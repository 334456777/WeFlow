// The sample's Moments: the release of WeFlow 6.0, the WebUI's new skin, a benchmark, ten years of records exported,
// an evening walk; the anniversary's flowers, a family dinner, calligraphy homework, table tennis and old roommates.
import type { SnsComment, SnsMedia, SnsPost } from '../api/contract.gen';
import { ago, at, daysSince } from './clock';
import { snsAddress, type Picture } from './media';
import { ME, nameOf } from './people';

const pictures = (...names: Picture[]): SnsMedia[] => names.map((name, i) => ({ url: snsAddress(name), thumb: snsAddress(name), key: String(i + 1) }));

const comment = (id: number, username: string, content: string, reply?: string): SnsComment => ({
  id: String(id), username, nickname: nameOf(username), content, refCommentId: '',
  ...(reply ? { refUsername: reply, refNickname: nameOf(reply) } : {}),
});

let count = 0;
function post(username: string, createTime: number, contentDesc: string, media: SnsMedia[], likes: string[] = [], comments: SnsComment[] = [], extra: Partial<SnsPost> = {}): SnsPost {
  count += 1;
  return {
    id: String(14_000_000_000_000 + count), tid: String(count), username, nickname: nameOf(username), createTime, contentDesc,
    type: media.length ? 1 : 2, rawXml: '', media, likes: likes.map(nameOf), comments, ...extra,
  };
}

const LIN = 'wxid_sample_linmu';
const CHEN = 'wxid_sample_chenyu';
const ZHOU = 'wxid_sample_zhouzhou';
const XU = 'wxid_sample_xuzhiyuan';
const HE = 'wxid_sample_hexi';
const SONG = 'wxid_sample_songxiaoman';
const GU = 'wxid_sample_gunan';
const LU = 'wxid_sample_xiaolu';
const MOM = 'wxid_sample_mom';
const DAD = 'wxid_sample_dad';
const AUNT = 'wxid_sample_aunt';
const YOUZI = 'wxid_sample_youzi';
const COACH = 'wxid_sample_coachzhou';
const AJIE = 'wxid_sample_ajie';
const QING = 'wxid_sample_qingqing';
const TEACHER = 'wxid_sample_teacherchen';
const JING = 'wxid_sample_jingguan';
const LAOMA = 'wxid_sample_laoma';
const BING = 'wxid_sample_dabing';

/** The timeline, newest first. */
export const timeline: SnsPost[] = [
  post(ME, ago(1, 1, 30), 'WeFlow 6.0 发布了！网页界面全面改版：新皮肤、深色模式，断开连接也不会丢掉正在看的记录。', pictures('release'),
    [LIN, CHEN, ZHOU, XU, HE, SONG, GU],
    [comment(1, LIN, '终于发了 🎉'), comment(2, CHEN, '性能那部分也写一写 😄'), comment(3, ME, '下一篇专门写', CHEN), comment(4, GU, '已经在用了，十年的记录都导出来了')]),
  post(LIN, ago(2, 0, 20), '给 WeFlow 画的新皮肤，浅色、深色和最早的草图', pictures('webui-light', 'webui-dark', 'skin-sketch'),
    [ME, ZHOU, XU, HE], [comment(5, ZHOU, '深色那版好看'), comment(6, ME, '下个版本就用它')]),
  post(CHEN, ago(3, 3, 0), '会话列表冷启动 380ms → 160ms。按页解密时少拷贝一次，收益比想象的大。', pictures('bench-chart'),
    [ME, LIN, SONG], [comment(7, SONG, '求分享火焰图'), comment(8, CHEN, '在 PR 里', SONG)]),
  post(ZHOU, ago(4, 5, 0), '把十年的聊天记录导出成了 HTML，原来我们聊了这么多', pictures('export-html'),
    [ME, GU, LIN], [comment(9, GU, '我也导了一份'), comment(10, ME, '记得再备份一份')], { location: { city: '北京', poiName: '家里' } }),
  post(HE, ago(5, 2, 0), '在 macOS 上取到图片密钥的那一刻，值得一杯咖啡。', [], [ME, CHEN, XU], [comment(11, ME, '辛苦了 ☕')]),
  post(ME, ago(6, 4, 0), '发版前最后一次线下讨论', pictures('meetup'), [LIN, CHEN, ZHOU, XU], [],
    { location: { city: '杭州', poiName: '街角咖啡', poiAddress: '滨江路 88 号' } }),
  post(XU, ago(7, 6, 0), 'WeFlow 的 Wiki 重写完了，从安装到原理都有，欢迎挑错', [{ url: 'https://github.com/334456777/WeFlow/wiki', thumb: snsAddress('logo'), key: '1' }],
    [ME, ZHOU], [comment(12, ZHOU, '「故障排查」那页太有用了')], { type: 3 }),
  post(SONG, ago(9, 3, 0), 'Linux 上的微信也能用 WeFlow 了', pictures('webui-dark'), [ME, HE]),
  post(ME, ago(12, 9, 0), '傍晚出去走走，脑子里还在想导出格式', pictures('street'), [LIN, GU, LU]),
  // the account's own life
  post(LU, at(1, '10:40'), '收到花了 🌷 某人这次真的没加班', pictures('flowers'), [ME, MOM, LIN, YOUZI, QING],
    [comment(13, MOM, '真好看'), comment(14, ME, '说到做到'), comment(15, LU, '这还差不多', ME)]),
  post(MOM, at(1, '19:40'), '试做了周末的菜，红烧肉管够', pictures('dinner'), [ME, DAD, AUNT, LU, YOUZI],
    [comment(16, DAD, '好吃'), comment(17, YOUZI, '给我留一块！')]),
  post(TEACHER, at(3, '10:20'), '学生的作业，一周比一周进步', pictures('calligraphy'), [ME, QING], [comment(18, ME, '谢谢老师 🙏')]),
  post(AJIE, at(6, '21:20'), '定了！毕业十年聚会，404 永不熄灯', [], [LAOMA, BING, ME], [comment(19, LAOMA, '我的黑历史全在阿远电脑里'), comment(20, ME, '只读的，放心', LAOMA)]),
  post(COACH, at(daysSince(3), '22:00'), '周三的球友们，下周继续', pictures('pingpong'), [AJIE, ME, QING], [comment(21, AJIE, '下周单挑阿远')]),
  post(JING, at(8, '06:30'), '此时此地。', [], [LU, ME, HE]),
].sort((a, b) => b.createTime - a.createTime);

/** Posts per poster. */
export function postCounts(): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const p of timeline) counts[p.username] = (counts[p.username] ?? 0) + 1;
  return counts;
}
