// 加载时的占位骨架屏（#282）：按将要出现的内容的形状画出占位块，上面有一道扫过的光，
// 这样等待 API 回答时页面保持布局，回答到了也不会跳动。减少动画时光不再扫动。
import type { ReactNode } from 'react';

/** 每行不同的宽度，像真实的名字和文字一样长短不一，每次渲染都相同。 */
const WIDTHS = [72, 54, 84, 46, 66, 58, 78, 50, 62, 70];
const width = (i: number, offset = 0) => WIDTHS[(i * 3 + offset) % WIDTHS.length] + '%';

function Wait({ label, className, children }: { label: string; className: string; children: ReactNode }) {
  return <div className={'skeleton-block ' + className} aria-busy="true" aria-label={label}>{children}</div>;
}

/** 像会话列表一样的若干行：每行一个圆片和两行文字。 */
export function ListSkeleton({ label, rows = 8, note }: { label: string; rows?: number; note?: string }) {
  return <Wait label={label} className="conversation-list list-skeleton">
    {note && <p className="skeleton-note">{note}</p>}
    {Array.from({ length: rows }, (_, i) => <div className="skeleton-row" key={i}>
      <span className="skeleton avatar" />
      <span><span className="skeleton line" style={{ width: width(i) }} /><span className="skeleton line short" style={{ width: width(i, 4) }} /></span>
    </div>)}
  </Wait>;
}

/** 消息：有些是对方发的、带头像，有些是自己发的，长短不一。 */
export function MessagesSkeleton({ label }: { label: string }) {
  const sides = [false, false, true, false, true, true, false];
  return <Wait label={label} className="messages messages-skeleton">
    {sides.map((outgoing, i) => <div className={'message-row ' + (outgoing ? 'outgoing' : 'incoming')} key={i}>
      {!outgoing && <span className="skeleton avatar" />}
      <span className="skeleton bubble" style={{ width: 'min(' + width(i, 2) + ', 360px)', height: i % 3 === 1 ? 56 : 36 }} />
    </div>)}
  </Wait>;
}

/** 朋友圈：每条有头像和几行文字，多数带一张图片。 */
export function MomentsSkeleton({ label }: { label: string }) {
  return <Wait label={label} className="moments moments-skeleton">
    {[0, 1, 2].map(i => <div className="moment" key={i}>
      <span className="skeleton avatar" />
      <span className="moment-skeleton-copy">
        <span className="skeleton line" style={{ width: '30%' }} />
        <span className="skeleton line" style={{ width: width(i) }} />
        <span className="skeleton line short" style={{ width: width(i, 5) }} />
        {i !== 1 && <span className="skeleton picture" />}
      </span>
    </div>)}
  </Wait>;
}

/** 卡片里的各项信息，按 `.info-grid` 的方式排列：每项一个标签和它的值。 */
export function GridSkeleton({ label, cells = 4 }: { label: string; cells?: number }) {
  return <Wait label={label} className="info-grid">
    {Array.from({ length: cells }, (_, i) => <span className="grid-skeleton-cell" key={i}>
      <span className="skeleton line label" /><span className="skeleton line" style={{ width: width(i, 1) }} />
    </span>)}
  </Wait>;
}

/** 会话的媒体，按 `.media-list` 的方式排列：每项一个缩略图和两行文字。 */
export function MediaSkeleton({ label, items = 4 }: { label: string; items?: number }) {
  return <Wait label={label} className="media-list">
    {Array.from({ length: items }, (_, i) => <span className="media-skeleton" key={i}>
      <span className="skeleton thumb" />
      <span><span className="skeleton line" style={{ width: width(i) }} /><span className="skeleton line short" style={{ width: width(i, 4) }} /></span>
    </span>)}
  </Wait>;
}

/** 搜索结果，像 `.search-result` 的行一样：会话、文字和时间。 */
export function ResultsSkeleton({ label, rows = 4 }: { label: string; rows?: number }) {
  return <Wait label={label} className="results-skeleton">
    {Array.from({ length: rows }, (_, i) => <span className="result-skeleton" key={i}>
      <span className="skeleton line label" /><span className="skeleton line" style={{ width: width(i, 2) }} /><span className="skeleton line time" />
    </span>)}
  </Wait>;
}

/** 微信支付记录，像某一天下面的 `.pay-card` 一样：头像、收款方和时间，以及金额。 */
export function PaymentsSkeleton({ label }: { label: string }) {
  return <Wait label={label} className="payments payments-skeleton">
    <span className="skeleton line day" />
    {[0, 1, 2, 3].map(i => <div className="pay-card" key={i}>
      <span className="skeleton avatar" />
      <span className="moment-skeleton-copy"><span className="skeleton line" style={{ width: width(i, 3) }} /><span className="skeleton line short" style={{ width: '30%' }} /></span>
      <span className="skeleton line amount" />
    </div>)}
  </Wait>;
}
