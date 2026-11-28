// 从 `weflow serve` 读到的一个会话的消息（#198）：最新的在最下面，向上滚动时加载更早的页，
// 更早的消息加到上方时屏幕上的位置不变。跳到某一天或某个搜索结果后（#199），窗口停在那里并标出那条消息，
// 向下滚动时加载更新的页。
import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { ArrowClockwise, ArrowDown, CircleNotch } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { LIVE_READ_MS, onLive, useGathered, useLive } from '../api/live';
import { getMembers, type GroupMember } from '../api/messages';
import { currentScope, onNewScope } from '../api/scope';
import { Avatar } from '../Avatar';
import { contentIn, FEEDBACK_MS } from '../motion';
import { dayOf, formatClock, formatDayHeading } from '../format';
import { kindOf } from '../messageKind';
import { isMine } from './messages';
import { MessageView, withGallery } from './MessageView';
import type { Preview } from '../MessageMedia';
import { kindOf as sessionKind } from '../sessionKind';
import { useMessages, type Focus, type Jump } from './useMessages';
import { SCROLL_INTENT } from '../Scrollbar';
import { MessagesSkeleton } from '../Skeleton';
import { t } from '../i18n';

/** 距顶部（底部）不到这么多像素时请求更早（更新）的消息。 */
const LOAD_AHEAD = 600;
/** 距底部在这么多像素以内时，窗口跟随新出现的内容（例如加载出来的图片）。 */
const STICK = 48;
/** 距底部超过这么多像素时，显示回到最新消息的按钮（#194）。 */
const FAR = 120;
/** 立即移动窗口：消息区为用户的滚动设了平滑滚动，不指定 `instant` 时这里的移动会变成缓慢的滑动。 */
const moveTo = (el: HTMLElement, top: number) => { if (el.scrollTo) el.scrollTo({ top, behavior: 'instant' }); else el.scrollTop = top; };
const NATIVE_ANCHORING = typeof CSS !== 'undefined' && Boolean(CSS.supports?.('overflow-anchor', 'auto'));

/** 把跳转的目标消息放到它的位置：搜索结果放在窗口中间，某一天的第一条消息放在靠近顶部处。 */
function place(el: HTMLElement, node: HTMLElement, focus: Focus) {
  const box = node.getBoundingClientRect();
  const top = el.scrollTop + box.top - el.getBoundingClientRect().top;
  moveTo(el, focus.align === 'center' ? top - (el.clientHeight - box.height) / 2 : top - 40);
}

interface Person { name: string; avatarUrl?: string }

const memberCache = new Map<string, Map<string, Person>>();
onNewScope(() => memberCache.clear());
// 与 `getMembers` 一样，成员保留到这个群来了实时消息、或者事件流中断后恢复为止（#201）。每次清掉都记一次数，
// 在清掉之前开始的读取，结果交给调用方，但不进缓存。
let forgets = 0;
let resumedAt = 0;
const forgotAt = new Map<string, number>();
onLive(event => {
  forgets++;
  if (event.type === 'resumed') {
    resumedAt = forgets;
    forgotAt.clear();
    memberCache.clear();
  } else if (sessionKind(event.sessionId) === 'group') {
    forgotAt.set(event.sessionId, forgets);
    memberCache.delete(event.sessionId);
  }
});

/**
 * 一个群的成员：每人在群里显示的名字和头像，与 `getMembers` 保留得一样久。读取与成员面板共用：
 * `signal` 被取消的调用方收到取消的错误、不再等待，读取本身继续进行。
 */
export async function loadMembers(chatroomId: string, signal?: AbortSignal): Promise<Map<string, Person>> {
  const kept = memberCache.get(chatroomId);
  if (kept) return kept;
  const scope = currentScope();
  const begun = forgets;
  const members = await getMembers(chatroomId);
  signal?.throwIfAborted();
  const map = new Map(members.map((m: GroupMember) => [m.username, { name: m.groupNickname || m.displayName || m.username, avatarUrl: m.avatarUrl || undefined }]));
  // 为之前的账号读的，或者在这个群的成员被清掉之前开始读的：交给调用方，不进缓存
  const current = scope === currentScope() && resumedAt <= begun && (forgotAt.get(chatroomId) ?? 0) <= begun;
  if (current) memberCache.set(chatroomId, map);
  return map;
}

const NOBODY = new Map<string, Person>();

/**
 * 群里每条消息的发送者：成员的名字和头像，按 `loadMembers` 的缓存。打开群时读取；
 * 群里来了实时消息或者事件流恢复后再读一次（可能有人进群或改了群昵称），连续的消息只读一次，
 * 新的名字到达之前继续显示原来的。
 */
function useMembers(chatroomId: string | null): Map<string, Person> {
  const [shown, setShown] = useState(() => ({ id: chatroomId, people: (chatroomId && memberCache.get(chatroomId)) || NOBODY }));
  const [round, setRound] = useState(0);
  const refresh = useGathered(() => setRound(n => n + 1), LIVE_READ_MS);
  useLive(event => {
    if (chatroomId && (event.type === 'resumed' || (event.type === 'message.new' && event.sessionId === chatroomId))) refresh();
  });
  useEffect(() => {
    if (!chatroomId) { setShown({ id: null, people: NOBODY }); return; }
    const kept = memberCache.get(chatroomId);
    if (kept) { setShown({ id: chatroomId, people: kept }); return; }
    const controller = new AbortController();
    loadMembers(chatroomId, controller.signal).then(people => setShown({ id: chatroomId, people }), () => { /* 保留已显示的名字，没有名字时显示用户名 */ });
    return () => controller.abort();
  }, [chatroomId, round]);
  // 读这个群的成员期间，不显示另一个群的名字
  return shown.id === chatroomId ? shown.people : NOBODY;
}

/** 缓存里一个群成员的名字，用于搜索结果的发送者；这个群的成员还没读到时（消息还没显示过）为 `undefined`。 */
export const memberName = (chatroomId: string, username: string) => memberCache.get(chatroomId)?.get(username)?.name;
/** 这个群的成员是否已经读到（在缓存里）。 */
export const membersKnown = (chatroomId: string) => memberCache.has(chatroomId);

/** 会话 `session` 的消息，从最新的开始，或者从 `jump` 指向的位置开始。 */
export function LiveMessages({ session, scroller, jump, onPreview }: {
  session: ChatSession; scroller: RefObject<HTMLElement | null>; jump: Jump | null; onPreview(preview: Preview): void;
}) {
  const id = session.username;
  const group = sessionKind(id) === 'group';
  const { thread, loadOlder, loadNewer, reload } = useMessages(id, jump);
  const people = useMembers(group ? id : null);
  const list = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  // 跳转后，图片、名字和更早的页陆续加载时目标消息保持在原位，直到用户滚动（与停在底部时一直显示最新消息一样）。
  const pinned = useRef<Focus | null>(null);
  const anchor = useRef<{ key: string; top: number } | null>(null);
  const shownFor = useRef<string | null>(null);
  const [marked, setMarked] = useState<string | null>(null);
  const focus = thread.focus;
  // 用户在上方阅读时下方来的实时消息（#201）：只计数，不滚动过去
  const [unseen, setUnseen] = useState(0);
  const [far, setFar] = useState(false);
  const tail = useRef<{ id: string; cursor: string | null } | null>(null);
  useEffect(() => {
    const last = thread.messages[thread.messages.length - 1]?._cursor ?? null;
    const before = tail.current;
    tail.current = { id, cursor: last };
    if (!before || before.id !== id) { setUnseen(0); return; }
    if (!before.cursor || last === before.cursor || thread.hasNewer || atBottom.current) return;
    const at = thread.messages.findIndex(m => m._cursor === before.cursor);
    if (at >= 0) setUnseen(n => n + thread.messages.length - 1 - at);
  }, [id, thread.messages, thread.hasNewer]);
  const toNewest = () => {
    const el = scroller.current;
    if (el) moveTo(el, el.scrollHeight);
    atBottom.current = true;
    pinned.current = null;
    setUnseen(0);
  };

  // 打开会话时显示最新的消息；跳转时显示目标消息（搜索结果在中间，某一天的第一条消息在顶部），并短暂地标出它。
  useLayoutEffect(() => {
    const el = scroller.current;
    const shown = id + ':' + (focus?.seq ?? 'newest');
    if (!el || thread.status !== 'ready' || shownFor.current === shown) return;
    shownFor.current = shown;
    anchor.current = null;
    const node = focus && list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(focus.cursor)}"]`);
    if (focus && node) {
      place(el, node, focus);
      atBottom.current = false;
      pinned.current = focus;
      setMarked(focus.cursor);
    } else {
      moveTo(el, el.scrollHeight);
      atBottom.current = true;
      pinned.current = null;
    }
  }, [id, thread.status, focus, thread.hasNewer, scroller]);

  // 没有缓存的会话在骨架屏之后才显示消息：面板的进入动画已经在骨架屏上播放过，所以消息自己再播放一次进入动画。
  // 有缓存的会话立即就绪，随面板的进入动画一起出现。
  const wasReady = useRef<{ id: string; ready: boolean } | null>(null);
  useEffect(() => {
    const before = wasReady.current;
    const ready = thread.status === 'ready';
    wasReady.current = { id, ready };
    if (ready && before && before.id === id && !before.ready && list.current) contentIn([list.current]);
  }, [id, thread.status]);

  useEffect(() => {
    if (!marked) return;
    const timer = window.setTimeout(() => setMarked(null), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [marked]);

  // 更早的消息加到了上方：让原来在最上面的消息留在原处。支持滚动锚定（scroll anchoring）的浏览器（Chrome、Edge、
  // Firefox）自己会做，上方的图片加载时也是，这里再修正一次就会移动两次。Safari 不支持；
  // 其他浏览器在窗口滚到最顶端时也不锚定，而快速甩动、拖动滚动条或按 Home 会在新的一页到达之前就滚到顶端（#159）。
  // 这两种情况由这里调整，调整同时让窗口离开顶端，继续向上滚动时才会请求下一页。
  useLayoutEffect(() => {
    const el = scroller.current;
    const kept = anchor.current;
    if (!el || !kept) return;
    const node = list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(kept.key)}"]`);
    if (node && node.dataset.key !== thread.messages[0]?._cursor) {
      if ((!NATIVE_ANCHORING || el.scrollTop === 0) && !pinned.current) moveTo(el, el.scrollTop + node.getBoundingClientRect().top - kept.top);
      anchor.current = null;
    }
  }, [thread.messages, scroller]);

  // 滚动到接近顶部时请求更早的消息；停在底部附近时，窗口跟随变长的内容。
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ask = () => {
      const first = thread.messages[0];
      const node = first && list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(first._cursor)}"]`);
      if (node) anchor.current = { key: first._cursor, top: node.getBoundingClientRect().top };
      loadOlder();
    };
    // 只有用户的操作（滚轮、触摸、按键、滚动条）才让窗口离开底部，图片加载使内容变长不算；滚回底部后又跟随底部。
    // 跳转到过去的位置后，下面还有更新的消息，这时的底部不是真正的底部：滚动到接近它时读取更新的消息。
    const below = () => el.scrollHeight - el.scrollTop - el.clientHeight;
    const idle = thread.status === 'ready' && !thread.loadingOlder && !thread.loadingNewer;
    const onScroll = () => {
      setFar(below() > FAR);
      if (!thread.hasNewer && !pinned.current && below() < STICK) {
        atBottom.current = true;
        setUnseen(0);
      }
      if (el.scrollTop < LOAD_AHEAD && idle && thread.hasMore) ask();
      else if (below() < LOAD_AHEAD && idle && thread.hasNewer) loadNewer();
    };
    const letGo = () => { atBottom.current = false; pinned.current = null; };
    const intents = ['wheel', 'touchmove', 'keydown', 'pointerdown', SCROLL_INTENT] as const;
    el.addEventListener('scroll', onScroll, { passive: true });
    for (const name of intents) el.addEventListener(name, letGo, { passive: true });
    // 第一页比窗口矮时无法滚动：立即请求更多。消息区隐藏时（手机上正在显示会话列表）高度为 0，这时不请求。
    if (idle && el.clientHeight > 0 && el.scrollHeight <= el.clientHeight + LOAD_AHEAD) {
      if (thread.hasMore) ask();
      else if (thread.hasNewer) loadNewer();
    }
    const hold = () => {
      const kept = pinned.current;
      const node = kept && list.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(kept.cursor)}"]`);
      if (kept && node) place(el, node, kept);
      else if (atBottom.current) moveTo(el, el.scrollHeight);
    };
    // 消息每次变化时也调整一次：固定在跳转目标期间，到达的更早的页不会把它挤开
    hold();
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(hold);
    if (list.current) resize?.observe(list.current);
    return () => {
      el.removeEventListener('scroll', onScroll);
      for (const name of intents) el.removeEventListener(name, letGo);
      resize?.disconnect();
    };
  }, [thread, loadOlder, loadNewer, scroller]);

  if (thread.status === 'idle' || thread.status === 'loading') {
    return <MessagesSkeleton label={t('正在读取消息')} />;
  }
  if (thread.status === 'error') {
    return <div className="pane-message" role="alert"><p>{thread.error}</p><button type="button" className="secondary-button" onClick={reload}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  }
  if (!thread.messages.length) return <div className="pane-message"><p>{t('这个会话没有消息。')}</p></div>;

  const now = new Date();
  const nameOf = (username: string | null) => (username && people.get(username)?.name) || username || '';
  return <div className="messages live" ref={list}>
    <div className="history-edge" aria-live="polite">{thread.loadingOlder
      ? <><CircleNotch size={14} className="spin" />{t('正在加载更早的消息')}</>
      : thread.error ? <><span>{thread.error}</span><button type="button" className="text-button" onClick={loadOlder}>{t('重试')}</button></>
        : thread.hasMore ? null : t('没有更早的消息了')}</div>
    {thread.messages.map((message, index) => {
      const day = dayOf(message.createTime);
      const newDay = index === 0 || dayOf(thread.messages[index - 1].createTime) !== day;
      const mine = isMine(message);
      const system = kindOf(message) === 'system';
      // 群里同一个人连续的几条消息，只在第一条上显示发送者的名字和头像。
      const previous = thread.messages[index - 1];
      const showSender = group && !mine && !system && (newDay || !previous || previous.senderUsername !== message.senderUsername || isMine(previous) || kindOf(previous) === 'system');
      const person = message.senderUsername ? people.get(message.senderUsername) : undefined;
      return <Fragment key={message._cursor}>
        {newDay && <div className="date-separator">{formatDayHeading(day, now)}</div>}
        {system
          ? <div className={'message-row system' + (marked === message._cursor ? ' highlighted' : '')} data-key={message._cursor}><MessageView sessionId={id} message={message} onPreview={preview => onPreview(withGallery(id, thread.messages, message) ?? preview)} /></div>
          : <div className={'message-row ' + (mine ? 'outgoing' : 'incoming') + (group && !mine ? ' with-avatar' : '') + (marked === message._cursor ? ' highlighted' : '')} data-key={message._cursor}>
            {group && !mine && (showSender ? <Avatar name={nameOf(message.senderUsername)} url={person?.avatarUrl} /> : <span className="avatar-gap" />)}
            <div className="message-body">
              {showSender && <span className="message-sender">{nameOf(message.senderUsername)}</span>}
              <MessageView sessionId={id} message={message} onPreview={preview => onPreview(withGallery(id, thread.messages, message) ?? preview)} />
            </div>
            <time className="message-time" dateTime={new Date(message.createTime * 1000).toISOString()}>{formatClock(message.createTime)}</time>
          </div>}
      </Fragment>;
    })}
    {thread.hasNewer && <div className="history-edge newer" aria-live="polite">{thread.loadingNewer ? <><CircleNotch size={14} className="spin" />{t('正在加载更新的消息')}</> : null}</div>}
    {thread.hasNewer && <div className="latest-bar"><button type="button" className="secondary-button as-button" onClick={reload}><ArrowDown size={14} />{t('回到最新消息')}</button></div>}
    {!thread.hasNewer && (unseen > 0 || far) && <div className="latest-bar"><button type="button" className="secondary-button as-button" onClick={toNewest}><ArrowDown size={14} />{unseen > 0 ? t('{n} 条新消息', { n: unseen }) : t('回到最新消息')}</button></div>}
  </div>;
}
