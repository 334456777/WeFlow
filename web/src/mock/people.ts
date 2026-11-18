// Who the sample is about: the account, the friends working on WeFlow and its WebUI with it, and the people of its own
// life (whom it loves, its family, the table tennis, calligraphy and reading groups, old roommates), the groups and the
// official accounts. Every name and WeChat ID here is made up.
import type { Contact } from '../api/contract.gen';

export interface Person {
  username: string;
  /** The name the list shows: the remark when there is one, else the nickname. */
  name: string;
  nickname: string;
  remark?: string;
  alias?: string;
  region?: string;
  labels?: string[];
  /** Their signature. */
  about?: string;
  /** What they often say, counted in their messages for a group member's statistics. */
  phrases?: string[];
  formerFriend?: boolean;
}

/** The account the sample is of. */
export const ME = 'wxid_sample_ayuan';

export const people: Person[] = [
  { username: ME, name: '阿远', nickname: '阿远', alias: 'ayuan_dev', region: '中国 杭州', about: '在写 WeFlow', phrases: ['好', '收到', '已经'] },
  { username: 'wxid_sample_linmu', name: '林沐', nickname: '沐沐', remark: '林沐', alias: 'linmu_design', region: '中国 上海', labels: ['WeFlow', '设计'], about: '正在给 WeFlow 画皮肤', phrases: ['皮肤', '阴影', '好看'] },
  { username: 'wxid_sample_chenyu', name: '陈屿', nickname: '屿', remark: '陈屿', alias: 'chenyu_rs', region: '中国 杭州', labels: ['WeFlow', 'Rust'], about: '在意每一毫秒', phrases: ['性能', '基准', '收到'] },
  { username: 'wxid_sample_zhouzhou', name: '周舟', nickname: '周舟', alias: 'zhouzhou', region: '中国 北京', labels: ['WeFlow', '测试'], about: '什么都想导出来看看', phrases: ['导出', '试试', '好了'] },
  { username: 'wxid_sample_xuzhiyuan', name: '许知远', nickname: '知远', remark: '许知远', region: '中国 成都', labels: ['WeFlow', '文档'], about: 'Wiki 写得比代码多', phrases: ['Wiki', '写进', '整理'] },
  { username: 'wxid_sample_hexi', name: '何夕', nickname: '何夕', region: '中国 深圳', labels: ['WeFlow', 'macOS'], about: '在 Mac 上验证一切', phrases: ['macOS', '测了', '密钥'] },
  { username: 'wxid_sample_songxiaoman', name: '宋小满', nickname: '小满', remark: '宋小满', region: '中国 南京', labels: ['WeFlow', 'Linux'], about: 'Linux 桌面用户', phrases: ['Linux', '跑起来了'] },
  { username: 'wxid_sample_gunan', name: '顾南', nickname: '顾南', region: '中国 广州', labels: ['朋友'], about: '想把十年的聊天记录存下来' },
  { username: 'wxid_sample_shenxinghe', name: '沈星河', nickname: '星河', region: '中国 武汉', labels: ['用户群'] },
  { username: 'wxid_sample_chenyiwei', name: '陈一苇', nickname: '一苇', region: '中国 西安', labels: ['用户群'] },
  { username: 'wxid_sample_tangguo', name: '唐果', nickname: '唐果', region: '中国 长沙', formerFriend: true },
  // the people of the account's own life
  { username: 'wxid_sample_xiaolu', name: '小鹿', nickname: '鹿', remark: '小鹿', region: '中国 杭州', labels: ['家人'], about: '今天也要早睡', phrases: ['早点睡', '哼', '这还差不多'] },
  { username: 'wxid_sample_mom', name: '妈妈', nickname: '幸福一家人', remark: '妈妈', region: '中国 湖州', labels: ['家人'], about: '家和万事兴', phrases: ['回家', '吃饭'] },
  { username: 'wxid_sample_dad', name: '爸爸', nickname: '岁月静好', remark: '爸爸', region: '中国 湖州', labels: ['家人'], about: '知足常乐' },
  { username: 'wxid_sample_aunt', name: '姑姑', nickname: '晓梅', remark: '姑姑', region: '中国 湖州', labels: ['家人'] },
  { username: 'wxid_sample_uncle', name: '二舅', nickname: '建国', remark: '二舅', region: '中国 苏州', labels: ['家人'], phrases: ['转发', '养生'] },
  { username: 'wxid_sample_youzi', name: '小柚', nickname: '一颗柚子', remark: '表妹小柚', region: '中国 南京', labels: ['家人'], about: '上岸了！' },
  { username: 'wxid_sample_coachzhou', name: '周教练', nickname: '老周乒乓', remark: '周教练', region: '中国 杭州', labels: ['乒乓球'], phrases: ['接龙', '反手'] },
  { username: 'wxid_sample_ajie', name: '阿杰', nickname: '杰', remark: '阿杰', region: '中国 杭州', labels: ['室友', '乒乓球'], about: '下周单挑', phrases: ['单挑', '奶茶'] },
  { username: 'wxid_sample_qingqing', name: '青青', nickname: '青青', region: '中国 杭州', labels: ['书法', '乒乓球'] },
  { username: 'wxid_sample_teacherchen', name: '陈老师', nickname: '墨缘', remark: '陈老师（书法）', region: '中国 杭州', labels: ['书法'], about: '上善若水', phrases: ['作业', '笔'] },
  { username: 'wxid_sample_jingguan', name: '静观', nickname: '静观', region: '中国 大理', labels: ['读书会'], about: '此时此地' },
  { username: 'wxid_sample_laoma', name: '老马', nickname: '马到成功', remark: '老马', region: '中国 成都', labels: ['室友'] },
  { username: 'wxid_sample_dabing', name: '大兵', nickname: '大兵', region: '中国 乌鲁木齐', labels: ['室友'] },
];

export const person = (username: string): Person | undefined => people.find(p => p.username === username);
export const nameOf = (username: string): string => person(username)?.name ?? username;

export interface Group {
  username: string;
  name: string;
  owner: string;
  members: string[];
  /** Folded into 「折叠的群聊」, and muted. */
  folded?: boolean;
  muted?: boolean;
}

export const DEV = 'sample_weflow_dev@chatroom';
export const DESIGN = 'sample_webui_design@chatroom';
export const USERS = 'sample_weflow_users@chatroom';
export const FAMILY = 'sample_family@chatroom';
export const PINGPONG = 'sample_pingpong@chatroom';
export const CALLIGRAPHY = 'sample_calligraphy@chatroom';
export const MINDFUL = 'sample_mindful@chatroom';
export const DORM = 'sample_dorm404@chatroom';

export const groups: Group[] = [
  {
    username: DEV, name: 'WeFlow 开发群', owner: ME,
    members: [ME, 'wxid_sample_chenyu', 'wxid_sample_linmu', 'wxid_sample_zhouzhou', 'wxid_sample_xuzhiyuan', 'wxid_sample_hexi', 'wxid_sample_songxiaoman'],
  },
  { username: DESIGN, name: 'WebUI 设计讨论', owner: 'wxid_sample_linmu', members: ['wxid_sample_linmu', ME, 'wxid_sample_xuzhiyuan', 'wxid_sample_zhouzhou'] },
  {
    username: USERS, name: 'WeFlow 用户交流群', owner: ME, folded: true, muted: true,
    members: [ME, 'wxid_sample_gunan', 'wxid_sample_shenxinghe', 'wxid_sample_chenyiwei', 'wxid_sample_songxiaoman', 'wxid_sample_hexi', 'wxid_sample_zhouzhou'],
  },
  {
    username: FAMILY, name: '相亲相爱一家人', owner: 'wxid_sample_mom',
    members: ['wxid_sample_mom', 'wxid_sample_dad', 'wxid_sample_aunt', 'wxid_sample_uncle', 'wxid_sample_youzi', ME, 'wxid_sample_xiaolu'],
  },
  {
    username: PINGPONG, name: '周三乒乓球', owner: 'wxid_sample_coachzhou', muted: true,
    members: ['wxid_sample_coachzhou', 'wxid_sample_ajie', 'wxid_sample_qingqing', 'wxid_sample_laoma', ME],
  },
  { username: CALLIGRAPHY, name: '墨池书法研习社', owner: 'wxid_sample_teacherchen', muted: true, members: ['wxid_sample_teacherchen', 'wxid_sample_qingqing', ME, 'wxid_sample_aunt'] },
  {
    username: MINDFUL, name: '觉知与当下 · 读书会', owner: 'wxid_sample_jingguan', folded: true, muted: true,
    members: ['wxid_sample_jingguan', 'wxid_sample_xiaolu', ME, 'wxid_sample_qingqing', 'wxid_sample_hexi'],
  },
  { username: DORM, name: '404 寝室（永不熄灯）', owner: 'wxid_sample_ajie', members: ['wxid_sample_ajie', 'wxid_sample_laoma', 'wxid_sample_dabing', ME] },
];

/** The conversations pinned to the top (#885): the WeFlow group, the family, the one loved and mother. */
export const pinned = new Set([DEV, FAMILY, 'wxid_sample_xiaolu', 'wxid_sample_mom']);

export interface Official {
  username: string;
  name: string;
  about: string;
}

export const WEFLOW_NEWS = 'gh_sample_weflow';
export const WXPAY = 'gh_sample_wxpay';

export const officials: Official[] = [
  { username: WEFLOW_NEWS, name: 'WeFlow 更新日志', about: 'WeFlow 的发布说明和使用技巧' },
  { username: 'gh_sample_rustnotes', name: 'Rust 编程手记', about: '写给 Rust 开发者的实践笔记' },
  { username: 'gh_sample_uidesign', name: '界面设计观察', about: '关于界面、交互和设计系统' },
  { username: 'gh_sample_calligraphy', name: '书法日课', about: '每天一个字' },
  { username: 'gh_sample_health', name: '健康生活小贴士', about: '养生知识，转发给家人' },
  { username: WXPAY, name: '微信支付', about: '微信支付的付款和退款通知' },
];

/** The name a conversation is listed by: the group's, the official account's, or the person's. */
export const sessionName = (username: string): string =>
  groups.find(g => g.username === username)?.name ?? officials.find(o => o.username === username)?.name
  ?? (username === 'filehelper' ? '文件传输助手' : nameOf(username));

/** The parsed contact list (`chat contacts --parsed`). */
export function contacts(): Contact[] {
  return [
    ...people.filter(p => p.username !== ME).map((p): Contact => ({
      username: p.username, displayName: p.name, remark: p.remark, nickname: p.nickname, alias: p.alias, labels: p.labels,
      detailDescription: p.about, region: p.region, type: p.formerFriend ? 'former_friend' : 'friend',
    })),
    ...groups.map((g): Contact => ({ username: g.username, displayName: g.name, nickname: g.name, type: 'group' })),
    ...officials.map((o): Contact => ({ username: o.username, displayName: o.name, nickname: o.name, detailDescription: o.about, type: 'official' })),
  ];
}

/** The contact table as the server reads it (`chat contacts`), for the names of Moments' posters. */
export function contactRows(): Record<string, unknown>[] {
  return [
    ...people.map(p => ({ username: p.username, nick_name: p.nickname, remark: p.remark ?? '', alias: p.alias ?? '' })),
    ...groups.map(g => ({ username: g.username, nick_name: g.name, remark: '' })),
    ...officials.map(o => ({ username: o.username, nick_name: o.name, remark: '' })),
  ];
}
