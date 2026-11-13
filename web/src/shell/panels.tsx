// The two shapes of a page: one that scrolls as a whole under its heading, and one of two sides, a list and what is
// picked from it, shown one at a time on a phone (#335).
import { useEffect, useRef } from 'react';
import type { ReactNode, RefObject } from 'react';
import type { Icon } from '@phosphor-icons/react';
import type { Backend } from '../api/backend';
import { MobileBar } from '../MobileNav';
import { contentIn } from '../motion';
import { Scrollbar } from '../Scrollbar';
import { TokenPrompt } from '../TokenForm';

export function ScrollPage({ label, icon: Glyph, tabs, scroller, children }: {
  label: string; icon: Icon; tabs?: ReactNode;
  /** For a page that scrolls itself back to the top. */
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

/** The records once connected; the token asked for when the server refuses this tab's, else what the page is for. */
export function Connected({ backend, offline, children }: { backend: Backend; offline: string; children: ReactNode }) {
  return backend.state === 'unauthorized' ? <TokenPrompt />
    : backend.state === 'ready' ? children
      : <p className="settings-footnote">{offline}</p>;
}

/** The same for the list side of a page of two sides. */
export function ConnectedList({ backend, offline, children }: { backend: Backend; offline: string; children: ReactNode }) {
  return backend.state === 'unauthorized' ? <TokenPrompt />
    : backend.state === 'ready' ? children
      : <div className="session-pane"><p className="session-message">{offline}</p></div>;
}

export function BrowserPanel({ label, className, detail, onBack, list, back, title, paneLabel, pane, detailKey, children }: {
  label: string;
  /** The page's own class, for the width of its list and the like. */
  className?: string;
  /** What the detail shows (a conversation, a contact, a section): another one comes in as content does (#776). */
  detailKey?: string | null;
  /** On a phone, the detail side is shown. */
  detail: boolean;
  onBack(): void;
  list: ReactNode;
  /** The phone's button back to the list, and the title beside it. */
  back: string; title: string;
  paneLabel: string;
  /** For a detail that scrolls with the pane. */
  pane?: RefObject<HTMLDivElement | null>;
  children?: ReactNode;
}) {
  const own = useRef<HTMLDivElement>(null);
  const ref = pane ?? own;
  // another page in this panel comes in whole (App), so only another item of the same page brings its detail in
  const shown = useRef({ label, detailKey });
  useEffect(() => {
    const before = shown.current;
    shown.current = { label, detailKey };
    if (before.label === label && before.detailKey !== detailKey && ref.current) contentIn(ref.current.children);
  }, [label, detailKey, ref]);
  return <section className={'panel browser-panel' + (className ? ' ' + className : '') + (detail ? ' show-detail' : '')} aria-label={label}>
    {list}
    <MobileBar label={back} title={title} onBack={onBack} />
    <div ref={ref} className="message-pane scroll-area" aria-label={paneLabel}>{children}</div>
    <Scrollbar target={ref} />
  </section>;
}
