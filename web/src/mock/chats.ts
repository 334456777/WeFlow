// The sample's conversations: friends and groups talking about WeFlow and its WebUI, and the account's own life (whom it
// loves, its family, table tennis, calligraphy, a reading group, old roommates), in every kind of message the chat
// window shows. Each is written as it happened, one message after another, its times counted back from now; what one
// chat says, another (or the Moments, the payments, the reports) often answers.
import type { ParsedMessage } from '../api/contract.gen';
import { ago, at, daysSince } from './clock';
import { pictureUrl, type Picture } from './media';
import { CALLIGRAPHY, DESIGN, DEV, DORM, FAMILY, ME, MINDFUL, PINGPONG, USERS } from './people';

export type Message = ParsedMessage & { _cursor: string };

/** A message as written below: who sent it, how many minutes after the one before, and what it is. */
interface Draft {
  from: string;
  gap: number;
  localType: number;
  text: string;
  extra?: Partial<ParsedMessage>;
}

/** An app message's type: its kind in the high 32 bits, 49 in the low ones. */
const app = (kind: number) => kind * 4_294_967_296 + 49;

const say = (from: string, text: string, gap = 1): Draft => ({ from, gap, localType: 1, text });
const pic = (from: string, picture: Picture, gap = 1): Draft => ({ from, gap, localType: 3, text: '[图片]', extra: { imageMd5: picture } });
const voice = (from: string, seconds: number, gap = 1): Draft => ({ from, gap, localType: 34, text: '[语音]', extra: { voiceDurationSeconds: seconds } });
const sticker = (from: string, gap = 1): Draft => ({ from, gap, localType: 47, text: '[动画表情]', extra: { emojiCdnUrl: pictureUrl('sticker') } });
const file = (from: string, fileName: string, fileSize: number, gap = 1): Draft => ({
  from, gap, localType: app(6), text: '[文件] ' + fileName,
  extra: { appMsgKind: 'file', fileName, fileSize, fileExt: fileName.split('.').pop() },
});
const link = (from: string, linkTitle: string, appMsgDesc: string, linkUrl: string, gap = 1, appMsgAppName = 'GitHub', thumb: Picture = 'logo'): Draft => ({
  from, gap, localType: app(5), text: '[链接] ' + linkTitle,
  extra: { appMsgKind: 'link', linkTitle, appMsgDesc, linkUrl, linkThumb: pictureUrl(thumb), appMsgAppName },
});
const quote = (from: string, text: string, quotedSender: string, quotedContent: string, gap = 1): Draft => ({
  from, gap, localType: app(57), text, extra: { appMsgKind: 'quote', quotedSender, quotedContent },
});
const record = (from: string, chatRecordTitle: string, lines: string[], gap = 1): Draft => ({
  from, gap, localType: app(19), text: '[聊天记录] ' + chatRecordTitle, extra: { appMsgKind: 'chat-record', chatRecordTitle, appMsgDesc: lines.join('\n') },
});
const transfer = (from: string, amount: string, note: string, gap = 1): Draft => ({
  from, gap, localType: app(2000), text: '[转账] ' + amount, extra: { appMsgKind: 'transfer', linkTitle: amount, appMsgDesc: note },
});
const redPacket = (from: string, wish: string, gap = 1): Draft => ({
  from, gap, localType: app(2001), text: '[红包] ' + wish, extra: { appMsgKind: 'red-packet', linkTitle: wish },
});
const card = (from: string, cardUsername: string, cardNickname: string, gap = 1): Draft => ({
  from, gap, localType: 42, text: '[名片] ' + cardNickname, extra: { cardUsername, cardNickname },
});
const place = (from: string, locationPoiname: string, locationLabel: string, gap = 1): Draft => ({
  from, gap, localType: 48, text: '[位置] ' + locationPoiname, extra: { locationPoiname, locationLabel },
});
const phone = (from: string, text: string, gap = 1): Draft => ({ from, gap, localType: 50, text });
const notice = (text: string, gap = 1): Draft => ({ from: '', gap, localType: 10000, text });

/** A conversation: from `start`, each message `gap` minutes after the one before; a number sets the time again. */
function thread(session: string, items: (Draft | number)[]): Message[] {
  let clock = 0;
  let localId = 0;
  const messages: Message[] = [];
  for (const item of items) {
    if (typeof item === 'number') { clock = item; continue; }
    localId += 1;
    // a few seconds into the minute, so that no two messages share a time
    clock += item.gap * 60 + (localId * 7) % 50;
    const mine = item.from === ME;
    messages.push({
      localId, serverId: String(7_100_000_000_000 + localId * 977), localType: item.localType, createTime: clock,
      sortSeq: clock * 1000 + localId, isSend: item.localType === 10000 ? 0 : mine ? 1 : 0,
      senderUsername: item.localType === 10000 ? null : item.from, parsedContent: item.text,
      rawContent: item.localType === 1 ? item.text : '', messageKey: session + ':' + localId, _cursor: 'c' + localId,
      ...item.extra,
    });
  }
  return messages;
}

const LIN = 'wxid_sample_linmu';
const CHEN = 'wxid_sample_chenyu';
const ZHOU = 'wxid_sample_zhouzhou';
const XU = 'wxid_sample_xuzhiyuan';
const HE = 'wxid_sample_hexi';
const SONG = 'wxid_sample_songxiaoman';
const GU = 'wxid_sample_gunan';
const SHEN = 'wxid_sample_shenxinghe';
const WEI = 'wxid_sample_chenyiwei';
const LU = 'wxid_sample_xiaolu';
const MOM = 'wxid_sample_mom';
const DAD = 'wxid_sample_dad';
const AUNT = 'wxid_sample_aunt';
const UNCLE = 'wxid_sample_uncle';
const YOUZI = 'wxid_sample_youzi';
const COACH = 'wxid_sample_coachzhou';
const AJIE = 'wxid_sample_ajie';
const QING = 'wxid_sample_qingqing';
const TEACHER = 'wxid_sample_teacherchen';
const JING = 'wxid_sample_jingguan';
const LAOMA = 'wxid_sample_laoma';
const BING = 'wxid_sample_dabing';

const REPO = 'https://github.com/334456777/WeFlow';
const WIKI = REPO + '/wiki';

/** The conversations by their username, each oldest first. */
export const threads = new Map<string, Message[]>([
  [DEV, thread(DEV, [
    ago(6, 9, 30),
    notice('"阿远"邀请"宋小满"加入了群聊'),
    say(SONG, '大家好，我主要用 Linux 版微信，可以帮忙测 Linux 这边', 2),
    say(ME, '欢迎！正缺人测 Linux'),
    say(XU, '6.0 的发版清单我列在 issue 里了，大家认领一下', 6),
    link(XU, '6.0 发版清单', '网页界面改版、导出格式补全、macOS 图片密钥、断开后保留记录', REPO + '/issues', 0),
    say(CHEN, 'weflow-native 那部分我来，WAL 解析的 Kani 证明昨晚跑完了', 3),
    say(LIN, 'WebUI 的皮肤还差深色模式的阴影', 2),
    say(ME, '好，我负责 release.yml 和 PGO 的训练清单', 1),
    say(ZHOU, '我这周把导出格式都过一遍：HTML、Excel、ChatLab、WeClone', 4),
    say(HE, 'macOS 的图片密钥我来测', 9),
    ago(5, 6, 10),
    pic(CHEN, 'bench-chart'),
    say(CHEN, '会话列表冷启动从 380ms 降到 160ms，主要是按页解密那里少了一次拷贝', 1),
    sticker(ME, 2),
    say(ME, '厉害，读消息那条也降了不少'),
    say(CHEN, '导出没怎么变，瓶颈在写文件', 3),
    say(HE, 'macOS 上图片密钥我用新版微信测了，能取到', 40),
    say(HE, '不过要先在微信里点开一张图片，不然内存里还没有密钥', 0),
    quote(XU, '这个要写进「故障排查」', '何夕', '不过要先在微信里点开一张图片，不然内存里还没有密钥', 5),
    say(XU, '已经加了，按现象列原因和处理办法', 20),
    ago(4, 3, 40),
    say(SONG, 'Linux 这边 serve 起来了，WebUI 能连上'),
    pic(SONG, 'webui-dark', 1),
    say(SONG, '深色模式在 Linux 上也正常', 0),
    say(LIN, '好看！侧栏底部的状态点可以再小一点', 6),
    say(ZHOU, '导出 HTML 的时候头像没出来，是我参数没加对吗', 30),
    say(ME, '要加 --media avatar，或者在网页的导出卡片里勾「头像」', 2),
    say(ZHOU, '好了', 5),
    pic(ZHOU, 'export-html', 0),
    say(ZHOU, '手机上打开也排得很好', 1),
    ago(3, 8, 20),
    file(XU, 'WeFlow 6.0 发布说明.md', 12_480),
    say(XU, '大家看看有没有漏的', 0),
    say(CHEN, 'API 版本这次要升吗', 12),
    say(ME, '不用，main 已经比发布版高一个版本了，改动并进这个版本，记到 Wiki 的「命令-API」里就行', 2),
    say(CHEN, '收到', 1),
    say(XU, '@阿远 release.yml 那边好了吗', 10),
    say(ME, '好了，今晚手动跑一次试试', 3),
    say(HE, 'macOS 的包签名我也看了，没问题', 12),
    say(LIN, '新皮肤的截图我晚点发到朋友圈', 50),
    ago(2, 10, 0),
    say(ME, '发版的 CI 全绿了 🎉'),
    say(HE, 'macOS 包我下下来试了，能打开', 4),
    voice(ZHOU, 6, 3),
    say(ZHOU, '刚才语音说的是：备份和恢复也测过了，没问题', 0),
    say(SONG, 'Linux 的压缩包也好了', 7),
    say(LIN, '我把新皮肤发到朋友圈了，欢迎点赞 😆', 30),
    say(XU, '已赞', 2),
    ago(1, 7, 30),
    say(CHEN, '有人遇到同一个浏览器开很多标签页就卡住的情况吗'),
    say(ME, '是 #799：看不见的页面过十秒会放开连接，新版已经修了', 3),
    say(CHEN, '了解', 1),
    record(ZHOU, '周舟和顾南的聊天记录', [
      '顾南：我想把十年的聊天记录存下来',
      '周舟：用 WeFlow 导出成 HTML，浏览器就能看',
      '顾南：图片和语音也能导出吗',
      '周舟：都能，语音会转成 MP3',
    ], 20),
    say(ZHOU, '用户问得最多的就是这个', 0),
    say(XU, '可以写一篇入门教程，我来', 4),
    say(ME, '好，放在 Wiki 的「WebUI 使用」里', 2),
    ago(0, 1, 0),
    say(ME, '晚上把 6.0.1 的修复合了，主要是断开连接后保留记录'),
    say(LIN, '这个好，之前一断开就变成示例数据，吓一跳', 2),
    say(CHEN, '@阿远 示例数据也该换换了，全是街景和咖啡', 1),
    say(ME, '已经在改了，改成讲 WeFlow 自己的 😄', 1),
    say(HE, '期待', 2),
    say(ZHOU, '那示例里的群是不是就是我们这个群', 30),
    say(ME, '你猜', 1),
  ])],

  [LIN, thread(LIN, [
    ago(3, 4, 0),
    pic(LIN, 'skin-sketch'),
    say(LIN, '新皮肤的草图画好了，按钮的外形层可以裁成不规则的形状', 1),
    say(ME, '这个思路好，roles 层把底色和阴影都交给 ::before 就行', 5),
    say(LIN, '对，页面只写布局和文字，不用管阴影', 2),
    ago(1, 5, 30),
    say(LIN, '深色模式下新拟态的阴影太重了'),
    say(ME, '可以把 --button-shadow-hover 调浅一点', 3),
    voice(LIN, 12, 2),
    say(LIN, '我还想加一个跟随系统的选项', 1),
    say(ME, '已经有了，在设置的「外观与浏览」里', 1),
    sticker(LIN, 1),
    ago(0, 0, 50),
    pic(LIN, 'webui-light'),
    say(LIN, '浅色这版怎么样', 0),
    say(ME, '控制台的卡片间距舒服多了', 4),
    say(LIN, '那我提 PR 了', 1),
    say(ME, '好，记得跑 bun run shots 前后对比一下截图', 1),
    say(LIN, '收到 👌', 1),
  ])],

  [ZHOU, thread(ZHOU, [
    ago(1, 2, 0),
    say(ZHOU, '请问 ChatLab 格式导出的文件能直接导入吗'),
    say(ME, '可以，选 ChatLab 或者 ChatLab JSONL 都行', 6),
    say(ZHOU, '那 WeClone 呢', 1),
    say(ME, 'WeClone 是给模型训练用的 CSV，一行一条消息', 2),
    quote(ZHOU, '明白了，我先导一个群试试', '阿远', 'WeClone 是给模型训练用的 CSV，一行一条消息', 1),
    ago(0, 1, 40),
    pic(ZHOU, 'export-html'),
    say(ZHOU, '导出来的 HTML 在手机上也能看', 0),
    transfer(ZHOU, '¥28.00', '请你喝咖啡，谢谢帮忙', 1),
    say(ME, '哈哈不用这么客气', 3),
    sticker(ZHOU, 1),
  ])],

  [DESIGN, thread(DESIGN, [
    ago(2, 9, 0),
    say(XU, '英文翻译是单独的 chunk 吧'),
    say(ME, '对，需要英文时才下载，首屏不带它', 2),
    say(LIN, '手机上底部导航要不要加一个「更多」', 5),
    say(ME, '已经有了，放不下的页面都在「更多」里', 1),
    say(ZHOU, '键盘用户呢', 3),
    say(ME, '每组一个 Tab 停留点，组内用方向键，按 / 搜索', 1),
    ago(0, 2, 50),
    say(LIN, '层叠层的顺序我整理了：tokens、app、pages、roles、skin'),
    say(XU, '写进 Wiki 的「设计语言」吧', 2),
    pic(LIN, 'webui-dark', 3),
    say(ZHOU, '示例数据要不要也换成 WeFlow 相关的', 6),
    say(ME, '要，聊天、朋友圈、支付都会换', 1),
    say(LIN, '支付也围绕 WeFlow？', 1),
    say(ME, '云服务器、域名、发版那晚的夜宵 😂', 1),
    say(XU, '哈哈很真实', 1),
  ])],

  [USERS, thread(USERS, [
    ago(2, 7, 0),
    notice('"沈星河"通过扫描二维码加入群聊'),
    say(SHEN, '请问 Windows 上取数据库密钥要管理员权限吗', 1),
    say(ME, '要，在管理员终端里运行 weflow key db，登录微信时就能取到', 4),
    say(WEI, '我用的是 macOS，怎么取', 2),
    say(HE, 'macOS 的步骤 Wiki 里有', 3),
    link(HE, '密钥获取 · WeFlow Wiki', '数据库密钥和图片密钥：各平台的前提、步骤和排障', WIKI, 0),
    ago(1, 4, 0),
    say(WEI, '@阿远 图片密钥取不到，提示内存里没有', 0),
    say(ME, '先在微信里点开一张图片再取', 3),
    say(GU, '导出的聊天记录可以按日期筛选吗', 20),
    say(ME, '可以，导出卡片里选时间范围', 2),
    redPacket(SHEN, '感谢解答', 3),
    say(WEI, '谢谢老板', 1),
    say(SONG, '周末有线下交流吗', 10),
    place(ME, '街角咖啡', '滨江路 88 号', 5),
    say(ME, '周六下午两点，这里见', 0),
    card(GU, CHEN, '陈屿', 8),
    say(GU, '这是群里的 Rust 大佬，性能问题可以找他', 0),
  ])],

  [GU, thread(GU, [
    ago(1, 9, 0),
    say(GU, '在吗，想请教个事'),
    say(GU, '我想把十年的微信聊天记录存到电脑上，有什么办法', 0),
    say(ME, '可以用 WeFlow，导出成 HTML，用浏览器就能看', 5),
    say(GU, '图片和语音也能保存吗', 1),
    say(ME, '能，导出时勾上图片和语音，语音会转成 MP3', 1),
    phone(GU, '[语音通话] 通话时长 18:24', 2),
    say(GU, '弄好了！谢谢', 30),
    say(GU, '我把我爸的也导出来了，他的聊天记录可多了', 0),
    say(ME, '记得再用「数据库备份」存一份', 3),
    sticker(GU, 1),
  ])],

  [CHEN, thread(CHEN, [
    ago(2, 3, 0),
    link(CHEN, 'perf: 按页解密少拷贝一次', 'weflow-native 解密时直接写进 SQLite 的页缓冲区', REPO + '/pulls'),
    say(CHEN, '帮忙 review 一下', 0),
    say(ME, '看了，unsafe 那段要补个 SAFETY 注释', 20),
    say(CHEN, '好', 1),
    file(CHEN, 'flamegraph.svg', 1_843_200, 6),
    ago(0, 5, 10),
    say(CHEN, '基准测试的机器今晚要关机维护'),
    say(ME, '那 bench 改到明天跑', 4),
    transfer(CHEN, '¥49.50', '云服务器分摊', 2),
    say(ME, '收到，谢谢', 3),
    say(CHEN, '下周想试试 PGO 对 serve 的效果', 1),
    say(ME, 'pgo-build.sh 的训练清单可以加上 WebUI 常用的几个读取', 2),
  ])],

  [HE, thread(HE, [
    ago(2, 6, 0),
    say(HE, '图片密钥在 macOS 上取到了'),
    say(HE, 'weflow key image 一下就出来了', 0),
    say(ME, '太好了，Linux 那边还差什么', 8),
    say(HE, '小满在测，WSL 里的 Linux 微信也能用', 2),
    say(ME, '那这周就能把三个平台都写进 Wiki', 1),
  ])],

  [XU, thread(XU, [
    ago(3, 5, 0),
    say(XU, 'Wiki 的「命令-API」页我按版本重新整理了'),
    link(XU, '命令-API · WeFlow Wiki', '路由、参数、作业和各版本的破坏性改动', WIKI, 0),
    say(ME, '好，破坏性改动每项写一句结果就行', 15),
    record(XU, 'WebUI 设计讨论的聊天记录', [
      '林沐：层叠层的顺序：tokens、app、pages、roles、skin',
      '阿远：按钮在各状态下的样子只写在 roles 层',
      '林沐：页面只用皮肤给的角色令牌',
    ], 30),
    say(XU, '这些我整理进「WebUI 实现」了', 0),
  ])],

  ['filehelper', thread('filehelper', [
    ago(3, 2, 0),
    file(ME, 'WeFlow 开发群.html', 2_411_520),
    file(ME, '聊天记录.xlsx', 865_280, 1),
    pic(ME, 'export-html', 1),
    file(ME, 'weflow-6.0.0-windows-x64.zip', 10_066_329, 30),
    file(ME, 'WebUI 设计稿.pdf', 3_250_585, 5),
    say(ME, '发版前检查：tag 用附注 tag，git cat-file -t 应该输出 tag', 2),
    at(2, '11:50'),
    say(ME, '备忘：纪念日记得订花 🌷，再订一家能看到湖的餐厅'),
  ])],

  [SONG, thread(SONG, [
    ago(5, 4, 0),
    say(SONG, 'Linux 版微信的数据目录在哪'),
    say(ME, '一般在 ~/Documents/xwechat_files 下面，Wiki 的「微信数据目录」有写', 6),
    say(SONG, '找到了，serve 跑起来了', 20),
    pic(SONG, 'webui-dark', 0),
  ])],

  // the account's own life
  [LU, thread(LU, [
    at(3, '17:40'),
    say(LU, '今晚吃什么'),
    say(ME, '你决定', 2),
    say(LU, '那就巷子里那家面', 1),
    place(LU, '巷子里的面馆', '梧桐巷 7 号', 0),
    at(2, '00:52'),
    say(LU, '都这么晚了还不睡'),
    say(ME, '发版前最后一个 bug', 3),
    say(LU, '你上次也这么说', 1),
    sticker(LU, 0),
    at(2, '08:30'),
    say(LU, '纪念日那天别加班啊'),
    say(ME, '早就请好假了', 4),
    say(LU, '这还差不多', 1),
    say(LU, '对了，你那个 WeFlow 能导出我们俩的聊天记录吗', 30),
    say(ME, '能，还能生成双人报告', 2),
    say(LU, '我们居然聊了三万多条', 6),
    say(LU, '年度报告里我排第几', 0),
    say(ME, '第一，一直是第一', 1),
    say(LU, '哼，算你识相 😤', 1),
    at(1, '23:20'),
    say(LU, '示例数据改得怎么样了'),
    say(ME, '在加我们的聊天记录', 2),
    say(LU, '那示例里的那个爱人是不是我', 1),
    say(ME, '除了你还能是谁', 1),
    say(LU, '…早点睡', 3),
    say(ME, '好，马上', 1),
    say(LU, '你上次也这么说', 1),
  ])],

  [MOM, thread(MOM, [
    at(6, '07:50'),
    voice(MOM, 59),
    voice(MOM, 42, 0),
    say(MOM, '天冷了，记得穿秋裤', 1),
    say(ME, '知道啦', 40),
    at(4, '20:10'),
    say(MOM, '你外婆的语音还在我手机的微信里，换手机会不会就没了'),
    say(ME, '不会，我用 WeFlow 帮你导出来存好', 6),
    say(MOM, 'WeFlow 是什么', 1),
    say(ME, '我在做的一个软件，能把微信的聊天记录存到电脑上', 2),
    say(MOM, '你自己做的？真厉害 👍', 1),
    at(3, '21:30'),
    file(ME, '外婆的语音.zip', 19_084_492),
    say(ME, '都转成 MP3 了，电脑和手机上都能听', 0),
    voice(MOM, 8, 25),
    say(MOM, '听到你外婆的声音，眼泪都下来了', 1),
    say(ME, '以后都不会丢了', 2),
    at(1, '19:20'),
    say(MOM, '周末回家吃饭吗'),
    say(MOM, '带上小鹿', 0),
    say(ME, '回！', 12),
    pic(MOM, 'dinner', 3),
    say(MOM, '先给你们看看菜单 😄', 0),
  ])],

  [FAMILY, thread(FAMILY, [
    at(5, '06:58'),
    pic(DAD, 'morning'),
    say(AUNT, '早上好', 4),
    say(UNCLE, '早', 6),
    at(3, '15:20'),
    say(YOUZI, '我考上研究生了！！！'),
    redPacket(AUNT, '恭喜小柚', 1),
    redPacket(UNCLE, '好样的', 1),
    redPacket(ME, '恭喜恭喜', 1),
    say(DAD, '👍👍👍', 2),
    say(MOM, '晚上一起吃饭庆祝', 3),
    at(2, '20:05'),
    say(UNCLE, '听说阿远会把微信聊天记录导出来？'),
    say(UNCLE, '那我们群里说的话是不是都被记下来了 😂', 0),
    say(AUNT, '[捂脸]', 2),
    say(ME, '只存在我自己的电脑上，放心，不会给别人看', 3),
    say(UNCLE, '那我以后发言注意点', 2),
    say(YOUZI, '二舅终于不转养生文章了吗', 1),
    at(1, '09:40'),
    link(UNCLE, '秋冬季节这样喝水最养生', '早上一杯温水，晚上少喝浓茶……', 'https://example.com/health', 0, '健康生活小贴士', 'morning'),
    say(YOUZI, '……', 5),
    ago(0, 0, 50),
    say(MOM, '周末都回家吃饭，我做红烧肉'),
    say(LU, '好的阿姨！', 3),
    say(YOUZI, '我也要来', 1),
    sticker(DAD, 4),
  ])],

  [PINGPONG, thread(PINGPONG, [
    at(daysSince(3) + 2, '12:10'),
    say(COACH, '本周三晚上七点，体育馆二楼，老规矩接龙'),
    say(AJIE, '#接龙\n周三乒乓球\n\n1. 周教练\n2. 阿杰', 3),
    say(ME, '#接龙\n周三乒乓球\n\n1. 周教练\n2. 阿杰\n3. 阿远', 5),
    say(QING, '#接龙\n周三乒乓球\n\n1. 周教练\n2. 阿杰\n3. 阿远\n4. 青青', 8),
    at(daysSince(3), '21:05'),
    pic(COACH, 'pingpong'),
    say(COACH, '今天阿远的反手拉得不错', 1),
    say(AJIE, '是不是写 Rust 练出来的，手腕灵活', 1),
    say(ME, '主要是对手发挥得好 😏', 2),
    say(AJIE, '？？？下周单挑', 1),
    say(COACH, '场地费一人 20，我先垫了', 6),
    transfer(ME, '¥20.00', '场地费', 2),
    at(daysSince(3), '22:30'),
    say(AJIE, '下周谁输了谁请奶茶'),
    say(ME, '再加一条：输的人帮我测一下 WeFlow', 3),
    say(AJIE, '那我宁愿请奶茶', 1),
    say(COACH, '哈哈哈哈', 2),
  ])],

  [CALLIGRAPHY, thread(CALLIGRAPHY, [
    at(8, '20:00'),
    say(TEACHER, '本周作业：临《兰亭序》前三行，周日前交'),
    say(QING, '收到', 5),
    say(ME, '收到', 20),
    at(4, '21:40'),
    pic(ME, 'calligraphy'),
    say(ME, '老师，这周的作业', 0),
    say(TEACHER, '结构比上周稳了。捺画再舒展一些，收笔不要急', 30),
    say(ME, '好的，谢谢老师', 2),
    say(TEACHER, '写字和做事一样，急不得', 1),
    say(QING, '阿远的字越来越有样子了', 6),
    at(3, '09:15'),
    say(TEACHER, '下周上课带毛毡和宣纸，墨我这里有'),
    say(ME, '老师，我做的那个软件叫 WeFlow，flow 就是流水的意思', 15),
    say(TEACHER, '难怪你的笔画越写越流畅 😄', 4),
    say(TEACHER, '上善若水', 0),
  ])],

  [MINDFUL, thread(MINDFUL, [
    at(9, '12:05'),
    say(JING, '今天的练习：吃饭的时候只吃饭，不看手机'),
    say(LU, '打卡 ✅', 30),
    say(ME, '我先把这条消息发完再开始', 1),
    say(JING, '🙏 觉察到想发消息的念头，也是练习', 3),
    at(4, '22:30'),
    say(ME, '最近把十年的聊天记录导出来看了一遍'),
    say(ME, '很多当时觉得天大的烦恼，现在都想不起来了', 0),
    say(JING, '记录会留下，烦恼会过去', 5),
    say(QING, '这句可以写成书法作品了', 2),
    say(LU, '那些说过的晚安也都还在吗', 4),
    say(ME, '都在', 1),
  ])],

  [DORM, thread(DORM, [
    at(10, '22:10'),
    say(AJIE, '毕业十年了，聚一下？'),
    say(LAOMA, '必须的', 2),
    say(BING, '我在外地，视频连线', 9),
    say(ME, '我把我们群这十年的聊天记录导出来了，三万多条', 5),
    say(LAOMA, '快把我当年那些黑历史删了', 1),
    say(ME, '删不了，WeFlow 是只读的 😂', 1),
    record(AJIE, '404 寝室的聊天记录', [
      '老马：今晚谁去图书馆占座',
      '大兵：我在网吧',
      '阿杰：熄灯了，别打游戏了',
      '老马：最后一局',
    ], 3),
    say(BING, '哈哈哈哈哈这都有', 1),
    at(6, '21:00'),
    say(AJIE, '定了，下周六老地方'),
    place(AJIE, '学校东门烧烤', '大学路 12 号', 0),
    redPacket(LAOMA, '聚会经费', 4),
  ])],
]);

/** How many messages each conversation has not been read, as the list shows. */
export const unread: Record<string, number> = {
  [DEV]: 3, [LIN]: 1, [ZHOU]: 2, [DESIGN]: 0, [USERS]: 6, [LU]: 1, [FAMILY]: 4, [CALLIGRAPHY]: 3, [MINDFUL]: 12, [DORM]: 5,
};

/** A message of a conversation by its local id. */
export const messageOf = (session: string, localId: number): Message | undefined =>
  threads.get(session)?.find(m => m.localId === localId);
