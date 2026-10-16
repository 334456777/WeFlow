// The messages of a conversation from `weflow serve` (#198): newest at the bottom, older pages loaded as the window
// scrolls up, with the place on screen kept while they arrive above.
import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { ArrowClockwise, CircleNotch } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { getMembers, type GroupMember } from '../api/messages';
import { Avatar } from '../Avatar';
import { dayOf, formatClock, formatDayHeading, isMine, kindOf } from './messages';
import { MessageView, type Preview } from './MessageView';
import { kindOf as sessionKind } from './sessions';
import { useMessages } from './useMessages';

/** Older messages are asked for while the top is closer than this. */
const LOAD_AHEAD = 600;
/** Within this of the bottom the window follows new content (pictures that load, for one). */
const STICK = 48;
/** Moves the window at once: the message pane scrolls smoothly for people, which would turn these into slow glides. */
const jump = (el: HTMLElement, top: number) => { if (el.scrollTo) el.scrollTo({ top, behavior: 'instant' }); else el.scrollTop = top; };
const NATIVE_ANCHORING = typeof CSS !== 'undefined' && Boolean(CSS.supports?.('overflow-anchor', 'auto'));

interface Person { name: string; avatarUrl?: string }

const memberCache = new Map<string, Map<string, Person>>();

/** Who said what in a group: names and pictures of its members, read once per group. */
function useMembers(chatroomId: string | null): Map<string, Person> {
  const [people, setPeople] = useState(() => (chatroomId && memberCache.get(chatroomId)) || new Map<string, Person>());
  useEffect(() => {
    if (!chatroomId) { setPeople(new Map()); return; }
    const kept = memberCache.get(chatroomId);
    if (kept) { setPeople(kept); return; }
    const controller = new AbortController();
    setPeople(new Map());
    getMembers(chatroomId, controller.signal).then(members => {
      const map = new Map(members.map((m: GroupMember) => [m.username, { name: m.groupNickname || m.displayName || m.username, avatarUrl: m.avatarUrl || undefined }]));
      memberCache.set(chatroomId, map);
      setPeople(map);
    }, () => { /* names fall back to the ids */ });
    return () => controller.abort();
  }, [chatroomId]);
  return people;
}

export function LiveMessages({ session, scroller, onPreview }: {
  session: ChatSession; scroller: RefObject<HTMLElement | null>; onPreview(preview: Preview): void;
}) {
  const id = session.username;
  const group = sessionKind(id) === 'group';
  const { thread, loadOlder, reload } = useMessages(id);
  const people = useMembers(group ? id : null);
  const list = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const anchor = useRef<{ key: string; top: number } | null>(null);
  const shownFor = useRef<string | null>(null);

  // Opening a conversation shows its newest messages.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || thread.status !== 'ready' || shownFor.current === id) return;
    shownFor.current = id;
    anchor.current = null;
    jump(el, el.scrollHeight);
    atBottom.current = true;
  }, [id, thread.status, scroller]);

  // Older messages arrived above: keep the message that was on top where it was. Browsers with scroll anchoring
  // (Chrome, Edge, Firefox) do it themselves, pictures loading above included; correcting it here as well would move
  // it twice. Safari has none, so it is done here.
  useLayoutEffect(() => {
    const el = scroller.current;
    const kept = anchor.current;
    if (!el || !kept) return;
    const node = list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(kept.key)}"]`);
    if (node && node.dataset.key !== thread.messages[0]?._cursor) {
      if (!NATIVE_ANCHORING) jump(el, el.scrollTop + node.getBoundingClientRect().top - kept.top);
      anchor.current = null;
    }
  }, [thread.messages, scroller]);

  // Scrolling near the top asks for older messages; near the bottom, the window follows growing content.
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ask = () => {
      const first = thread.messages[0];
      const node = first && list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(first._cursor)}"]`);
      if (node) anchor.current = { key: first._cursor, top: node.getBoundingClientRect().top };
      loadOlder();
    };
    // Only the person lets go of the bottom (wheel, touch, keys, the scrollbar); pictures that load and grow the
    // content do not, and scrolling back down holds it again.
    const onScroll = () => {
      if (el.scrollHeight - el.scrollTop - el.clientHeight < STICK) atBottom.current = true;
      if (el.scrollTop < LOAD_AHEAD && thread.status === 'ready' && thread.hasMore && !thread.loadingOlder) ask();
    };
    const letGo = () => { atBottom.current = false; };
    const intents = ['wheel', 'touchmove', 'keydown', 'pointerdown'] as const;
    el.addEventListener('scroll', onScroll, { passive: true });
    for (const name of intents) el.addEventListener(name, letGo, { passive: true });
    // A first page shorter than the window cannot be scrolled: ask for more at once.
    // Not while the pane is hidden (the list's turn on a phone), when it has no height at all.
    if (thread.status === 'ready' && thread.hasMore && !thread.loadingOlder && el.clientHeight > 0 && el.scrollHeight <= el.clientHeight + LOAD_AHEAD) ask();
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => { if (atBottom.current) jump(el, el.scrollHeight); });
    if (list.current) resize?.observe(list.current);
    return () => {
      el.removeEventListener('scroll', onScroll);
      for (const name of intents) el.removeEventListener(name, letGo);
      resize?.disconnect();
    };
  }, [thread, loadOlder, scroller]);

  if (thread.status === 'idle' || thread.status === 'loading') {
    return <div className="pane-message" aria-busy="true"><CircleNotch size={20} className="spin" /><p>正在读取消息</p></div>;
  }
  if (thread.status === 'error') {
    return <div className="pane-message" role="alert"><p>{thread.error}</p><button type="button" className="secondary-button" onClick={reload}><ArrowClockwise size={14} />重试</button></div>;
  }
  if (!thread.messages.length) return <div className="pane-message"><p>这个会话没有消息。</p></div>;

  const now = new Date();
  const nameOf = (username: string | null) => (username && people.get(username)?.name) || username || '';
  return <div className="messages live" ref={list}>
    <div className="history-edge" aria-live="polite">{thread.loadingOlder
      ? <><CircleNotch size={14} className="spin" />正在加载更早的消息</>
      : thread.error ? <><span>{thread.error}</span><button type="button" className="text-button" onClick={loadOlder}>重试</button></>
        : thread.hasMore ? null : '没有更早的消息了'}</div>
    {thread.messages.map((message, index) => {
      const day = dayOf(message.createTime);
      const newDay = index === 0 || dayOf(thread.messages[index - 1].createTime) !== day;
      const mine = isMine(message);
      const system = kindOf(message) === 'system';
      // In a group, the sender's name and picture on the first of their messages in a row.
      const previous = thread.messages[index - 1];
      const showSender = group && !mine && !system && (newDay || !previous || previous.senderUsername !== message.senderUsername || isMine(previous) || kindOf(previous) === 'system');
      const person = message.senderUsername ? people.get(message.senderUsername) : undefined;
      return <Fragment key={message._cursor}>
        {newDay && <div className="date-separator">{formatDayHeading(day, now)}</div>}
        {system
          ? <div className="message-row system" data-key={message._cursor}><MessageView sessionId={id} message={message} onPreview={onPreview} /></div>
          : <div className={'message-row ' + (mine ? 'outgoing' : 'incoming') + (group && !mine ? ' with-avatar' : '')} data-key={message._cursor}>
            {group && !mine && (showSender ? <Avatar name={nameOf(message.senderUsername)} url={person?.avatarUrl} /> : <span className="avatar-gap" />)}
            <div className="message-body">
              {showSender && <span className="message-sender">{nameOf(message.senderUsername)}</span>}
              <MessageView sessionId={id} message={message} onPreview={onPreview} />
            </div>
            <time className="message-time" dateTime={new Date(message.createTime * 1000).toISOString()}>{formatClock(message.createTime)}</time>
          </div>}
      </Fragment>;
    })}
  </div>;
}
