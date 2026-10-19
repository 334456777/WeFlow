// Loading placeholders (#282): the shape of what is coming, as shallow hollows with a sweep of light, so the page
// keeps its layout while the API answers and nothing jumps when it does. The sweep stops with reduced motion.
import type { ReactNode } from 'react';

/** Widths that vary from row to row like real names and lines, the same on every render. */
const WIDTHS = [72, 54, 84, 46, 66, 58, 78, 50, 62, 70];
const width = (i: number, offset = 0) => WIDTHS[(i * 3 + offset) % WIDTHS.length] + '%';

function Wait({ label, className, children }: { label: string; className: string; children: ReactNode }) {
  return <div className={'skeleton-block ' + className} aria-busy="true" aria-label={label}>{children}</div>;
}

/** A list of rows like the conversation list: a disc and two lines. */
export function ListSkeleton({ label, rows = 8, note }: { label: string; rows?: number; note?: string }) {
  return <Wait label={label} className="conversation-list list-skeleton">
    {note && <p className="skeleton-note">{note}</p>}
    {Array.from({ length: rows }, (_, i) => <div className="skeleton-row" key={i}>
      <span className="skeleton avatar" />
      <span><span className="skeleton line" style={{ width: width(i) }} /><span className="skeleton line short" style={{ width: width(i, 4) }} /></span>
    </div>)}
  </Wait>;
}

/** Messages, some from the other side with a face, some sent, of different lengths. */
export function MessagesSkeleton({ label }: { label: string }) {
  const sides = [false, false, true, false, true, true, false];
  return <Wait label={label} className="messages messages-skeleton">
    {sides.map((outgoing, i) => <div className={'message-row ' + (outgoing ? 'outgoing' : 'incoming')} key={i}>
      {!outgoing && <span className="skeleton avatar" />}
      <span className="skeleton bubble" style={{ width: 'min(' + width(i, 2) + ', 360px)', height: i % 3 === 1 ? 56 : 36 }} />
    </div>)}
  </Wait>;
}

/** Moments: a face, a few lines and a picture each. */
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

/** Facts of a card, as `.info-grid` lays them out: a label and its value each. */
export function GridSkeleton({ label, cells = 4 }: { label: string; cells?: number }) {
  return <Wait label={label} className="info-grid">
    {Array.from({ length: cells }, (_, i) => <span className="grid-skeleton-cell" key={i}>
      <span className="skeleton line label" /><span className="skeleton line" style={{ width: width(i, 1) }} />
    </span>)}
  </Wait>;
}

/** Media of a conversation, as `.media-list` lays them out: a thumbnail and two lines each. */
export function MediaSkeleton({ label, items = 4 }: { label: string; items?: number }) {
  return <Wait label={label} className="media-list">
    {Array.from({ length: items }, (_, i) => <span className="media-skeleton" key={i}>
      <span className="skeleton thumb" />
      <span><span className="skeleton line" style={{ width: width(i) }} /><span className="skeleton line short" style={{ width: width(i, 4) }} /></span>
    </span>)}
  </Wait>;
}

/** Hits of a search: the conversation, the text and the time, as `.search-result` rows. */
export function ResultsSkeleton({ label, rows = 4 }: { label: string; rows?: number }) {
  return <Wait label={label} className="results-skeleton">
    {Array.from({ length: rows }, (_, i) => <span className="result-skeleton" key={i}>
      <span className="skeleton line label" /><span className="skeleton line" style={{ width: width(i, 2) }} /><span className="skeleton line time" />
    </span>)}
  </Wait>;
}

/** WeChat Pay records, as `.pay-card`s under a day: a face, the payee and the time, the amount. */
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
