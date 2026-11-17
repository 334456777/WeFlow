// The sample's Moments: the release of WeFlow 6.0, the WebUI's new skin, a benchmark, ten years of records exported,
// and an evening walk.
import type { SnsComment, SnsMedia, SnsPost } from '../api/contract.gen';
import { ago } from './clock';
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
  post(ME, ago(12, 9, 0), '傍晚出去走走，脑子里还在想导出格式', pictures('street'), [LIN, GU]),
];

/** Posts per poster. */
export function postCounts(): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const p of timeline) counts[p.username] = (counts[p.username] ?? 0) + 1;
  return counts;
}
