// 工作区交给每个页面的内容，以及页面交回的内容（#401）。页面自己保存状态，所以打开别的页面再回来时保持原样；
// 工作区决定显示哪个页面，并汇集操作控制台的卡片。
import type { ReactNode } from 'react';
import type { Backend, Health } from '../api/backend';
import type { ChatSession } from '../api/chat';
import type { Profile } from '../api/setup';
import type { Card } from '../CardStack';
import type { Preview } from '../MessageMedia';
import type { SessionsState } from '../useSessions';
import type { JobsState } from '../jobs/useJobs';

/** 所有页面共用的内容：服务端、为所有页面读一次的记录，以及屏幕的各个部分。 */
export interface Shell {
  backend: Backend;
  /** 服务端现在在回答。 */
  connected: boolean;
  /**
   * 会改动东西或跟随改动的功能（作业、设置）可以使用：服务端在回答，或者示例服务端代替它回答（#896），
   * 改动保存在页面里。
   */
  usable: boolean;
  /** `usable` 所指的服务端的 `/health`：服务端的或示例服务端的；都没有时为 null。 */
  health: Health | null;
  /**
   * 网页显示服务端的记录（`holdsRecords`）：已连接，或者连接过又断开。断开时继续显示已读到的记录，还没读的照常请求
   * （并失败，说明连不上服务端），直到服务端回来。
   */
  held: boolean;
  /**
   * 网页显示记录（`recordsOf`）：服务端的（`held`），或者示例的（`sample`，由 src/mock 的示例服务端回答网页的读取，
   * #879）。
   */
  records: boolean;
  /** 记录是示例的：还没有服务端回答过这个网页，或者现在的服务端不适用于它。 */
  sample: boolean;
  /** 作用域：换了服务端或账号时变化（#198），页面为之前的作用域读到的内容随之作废。 */
  scope: number;
  /** 会话列表，其他页面用它显示会话的名字、从它打开会话。 */
  sessions: SessionsState;
  sessionList: ChatSession[];
  reloadSessions(): void;
  jobs: JobsState;
  profile: Profile | undefined;
  /** 手机上有两栏的页面一次只显示一栏：选了某一项之后显示详情（#335）。 */
  mobileDetail: boolean;
  setMobileDetail(show: boolean): void;
  openPreview(preview: Preview): void;
  toast(message: string): void;
}

/** 打开聊天页的某个会话。 */
export type ChatWith = (username: string) => void;
/** 打开聊天页，定位到某个会话的某条消息。 */
export type OpenAt = (sessionId: string, localId: number, createTime: number) => void;

/** 页面交给工作区的内容。 */
export interface Page {
  /** 页面在导航旁边显示的内容；没有记录可显示时不给（这时显示聊天页的）。 */
  view?: ReactNode;
  /** 页面自己在操作控制台里的卡片，不论是否显示都给出：操作控制台为全部卡片保持同一个顺序。 */
  cards?: Card[];
  /** 页面显示的卡片，包括自己的和共用的（如 `'tasks'`）；与 `view` 一样可以不给。 */
  shown?: string[];
}
