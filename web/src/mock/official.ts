// 示例数据的公众号和它们的推送：WeFlow 的发布说明、Rust 和界面设计的文章、每天一课的书法，以及家人转发的养生贴士；
// 微信支付公众号推送支付通知。
import type { BizMessage } from '../api/contract.gen';
import { ago } from './clock';
import { pictureUrl } from './media';
import { payRecords } from './pay';
import { WEFLOW_NEWS, WXPAY } from './people';

const REPO = 'https://github.com/334456777/WeFlow';

interface Article { title: string; summary: string; url: string; cover: string }

const article = (title: string, summary: string, url = REPO, cover = pictureUrl('logo')): Article => ({ title, summary, url, cover });

function push(localId: number, createTime: number, lead: Article, more: Article[] = []): BizMessage {
  return {
    localId, createTime, title: lead.title, description: lead.summary, url: lead.url, cover: lead.cover,
    contentList: [lead, ...more].map(({ title, url, cover, summary }) => ({ title, url, cover, summary })),
  };
}

/** 每个公众号的推送，最新的在前。 */
export const pushes = new Map<string, BizMessage[]>([
  [WEFLOW_NEWS, [
    push(31, ago(1, 2, 0), article('WeFlow 6.0 发布：网页界面全面改版', '新拟态皮肤、深色模式、断开连接后保留已读到的记录', REPO + '/releases', pictureUrl('release')), [
      article('WebUI 的皮肤是怎么做的', '层叠层、角色令牌和可裁剪的外形层', REPO + '/wiki', pictureUrl('skin-sketch')),
      article('从 SQLCipher 到纯 Rust 的只读数据库层', '按页解密，不再依赖闭源的 wcdb_api', REPO + '/wiki', pictureUrl('bench-chart')),
    ]),
    push(30, ago(8, 3, 0), article('导出格式一览：HTML、Excel、ChatLab、WeClone', '每种格式适合做什么，附带的媒体放在哪里', REPO + '/wiki', pictureUrl('export-html'))),
    push(29, ago(15, 3, 0), article('在 macOS 上获取密钥', '数据库密钥和图片密钥的前提、步骤和常见问题', REPO + '/wiki')),
    push(28, ago(22, 3, 0), article('在浏览器里看聊天记录：weflow serve 入门', '启动服务、打开网页、第一次配置账号', REPO + '/wiki', pictureUrl('webui-light'))),
  ]],
  ['gh_sample_rustnotes', [
    push(12, ago(4, 1, 0), article('按页解密：一个只读的 SQLite VFS', '读一页解一页，缓存命中时不再拷贝', REPO, pictureUrl('bench-chart'))),
    push(11, ago(11, 1, 0), article('用 Kani 证明 WAL 解析不越界', '把输入的上限写进证明的注释里', REPO)),
  ]],
  ['gh_sample_uidesign', [
    push(8, ago(6, 1, 0), article('新拟态之后：可裁剪的外形层', '按钮的底色和阴影交给 ::before，页面只写布局', REPO, pictureUrl('skin-sketch'))),
    push(7, ago(13, 1, 0), article('层叠层让样式不再互相覆盖', 'tokens、app、pages、roles、skin，后面的层总是胜出', REPO, pictureUrl('webui-dark'))),
  ]],
  ['gh_sample_calligraphy', [
    push(52, ago(0, 7, 0), article('每日一字：永', '「永」字八法，一个字里有楷书的八种基本笔画', 'https://example.com/calligraphy', pictureUrl('calligraphy'))),
    push(51, ago(1, 7, 0), article('临帖的第一年，先求像，再求不像', '读帖、摹帖、临帖、背帖，一步一步来', 'https://example.com/calligraphy')),
  ]],
  ['gh_sample_health', [
    push(77, ago(1, 13, 0), article('秋冬季节这样喝水最养生', '早上一杯温水，晚上少喝浓茶……', 'https://example.com/health', pictureUrl('morning'))),
  ]],
  [WXPAY, payRecords.slice(0, 6).map((record, i) => ({
    localId: 900 - i, createTime: record.timestamp, title: record.title, description: record.description, url: '', cover: '', contentList: [],
  }))],
]);
