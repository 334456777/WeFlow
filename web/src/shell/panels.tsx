// 页面的两种形状：一种在标题下整体滚动；另一种有两栏，一栏是列表，一栏是从中选中的项，手机上一次只显示一栏（#335）。
import { useEffect, useRef } from 'react';
import type { ReactNode, RefObject } from 'react';
import type { Icon } from '@phosphor-icons/react';
import type { Backend } from '../api/backend';
import { recordsOf } from '../api/records';
import { MobileBar } from '../MobileNav';
import { pageIn } from '../motion';
import { Scrollbar } from '../Scrollbar';
import { TokenPrompt } from '../TokenForm';
import { SampleNote } from './sample';

/** 只有一栏、整体滚动的页面：带图标和标签页的标题，然后是 `children`。 */
export function ScrollPage({ label, icon: Glyph, tabs, scroller, children }: {
  label: string; icon: Icon; tabs?: ReactNode;
  /** 页面要自己滚回顶部时传入。 */
  scroller?: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  const own = useRef<HTMLElement>(null);
  const ref = scroller ?? own;
  return <><section ref={ref} className="panel browser-panel settings-panel scroll-area" aria-label={label}>
    <div className="settings-heading"><Glyph size={20} /><h1>{label}</h1>{tabs}</div>
    {children}
  </section><Scrollbar target={ref} /></>;
}

/**
 * 页面的记录（`recordsOf`）：服务端的、服务端断开时之前读到的，或者示例的（并注明是示例）；服务端拒绝这个标签页的
 * token 时要求输入 token；还在寻找服务端时，显示这个页面是做什么的（`offline`）。
 */
export function Connected({ backend, offline, children }: { backend: Backend; offline: string; children: ReactNode }) {
  return backend.state === 'unauthorized' ? <TokenPrompt />
    : recordsOf(backend) ? <><SampleNote />{children}</>
      : <p className="settings-footnote">{offline}</p>;
}

/** 与 `Connected` 相同，用于两栏页面的列表一栏；列表是示例时由它自己说明（`SampleNote`）。 */
export function ConnectedList({ backend, offline, children }: { backend: Backend; offline: string; children: ReactNode }) {
  return backend.state === 'unauthorized' ? <TokenPrompt />
    : recordsOf(backend) ? children
      : <div className="session-pane"><p className="session-message">{offline}</p></div>;
}

/** 两栏的页面：列表，以及其中打开的那一项的详情；窄屏上一次只显示一栏。 */
export function BrowserPanel({ label, className, detail, onBack, list, back, title, paneLabel, pane, detailKey, children }: {
  label: string;
  /** 页面自己的类名，用来设置列表的宽度等。 */
  className?: string;
  /** 详情显示的是什么（一个会话、一个联系人、一个分区）：换成另一个时，像换页一样播放进入动画（`pageIn`，#776、
   * #832）。 */
  detailKey?: string | null;
  /** 手机上显示详情一栏。 */
  detail: boolean;
  onBack(): void;
  list: ReactNode;
  /** 手机上返回列表的按钮，以及它旁边的标题。 */
  back: string; title: string;
  paneLabel: string;
  /** 详情要用到滚动的面板时传入（如聊天页的消息）。 */
  pane?: RefObject<HTMLDivElement | null>;
  children?: ReactNode;
}) {
  const own = useRef<HTMLDivElement>(null);
  const ref = pane ?? own;
  // 换页时整个页面播放进入动画（App），所以这里只在同一页面换了一项时让详情播放进入动画
  const shown = useRef({ label, detailKey });
  useEffect(() => {
    const before = shown.current;
    shown.current = { label, detailKey };
    // 原来没有详情（页面打开时选中第一项）时，详情播放进入动画，但不算换页
    if (before.label === label && before.detailKey !== detailKey && ref.current) pageIn(ref.current.children, { turn: Boolean(before.detailKey), detail: true });
  }, [label, detailKey, ref]);
  return <section className={'panel browser-panel' + (className ? ' ' + className : '') + (detail ? ' show-detail' : '')} aria-label={label}>
    {list}
    <MobileBar label={back} title={title} onBack={onBack} />
    <div ref={ref} className="message-pane scroll-area" aria-label={paneLabel}>{children}</div>
    <Scrollbar target={ref} />
  </section>;
}
