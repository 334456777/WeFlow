// 「原图」 in the preview's lower left corner (#194, #341, #766): WeChat keeps up to three files of a picture: the
// thumbnail a chat shows, the image shown when it is opened, which the preview shows, and the original it downloads on
// View original. When the original is not on this computer and the server can have WeChat download it, the button asks
// for it as the preview opens and waits for the file, then shows it; pressed again it shows the image opened (#338).
import { useEffect, useRef, useState } from 'react';
import { CircleNotch, FrameCorners } from '@phosphor-icons/react';
import { getImageInfo, requestOriginal, type ImageInfo } from '../api/messages';
import { formatFileSize } from './messages';
import type { PreviewHd } from './MessageView';
import { t } from '../i18n';

/** How often, and for how long, a requested original is looked for: WeChat downloads it at its next download, which
 * may be well after the request. */
const POLL_MS = 2000;
const POLL_TRIES = 30;

type State =
  | { status: 'loading' }
  | { status: 'ready'; info: ImageInfo }
  /** Asked the server: WeChat downloads it at its next download. */
  | { status: 'downloading'; info: ImageInfo }
  /** Asking failed: the server could not leave the request. Pressing the button asks again. */
  | { status: 'failed'; info: ImageInfo }
  /** Waited a minute: WeChat has not downloaded it yet. Pressing the button asks again. */
  | { status: 'waiting'; info: ImageInfo }
  | { status: 'error' };

/** Whether pressing the button asks the server again instead of showing the original. */
const again = (state: State): boolean => state.status === 'failed' || state.status === 'waiting';

/** What the button says when it cannot be pressed, or what pressing it does again, or null when it shows the original. */
export function originalHint(state: State): string | null {
  if (state.status === 'loading') return t('正在查看这台电脑上有没有高清原图');
  if (state.status === 'error') return t('没能查到这张图片有没有高清原图');
  if (state.status === 'downloading') return t('正在下载原图：微信下次下载图片时会顺带下载这张，请稍等');
  if (state.status === 'failed') return t('没能让微信下载原图，点此重试');
  if (state.status === 'waiting') return t('微信还没有下载原图，点此重试');
  const { info } = state;
  if (info.hd) return null;
  if (!info.hdBytes) return t('这张图片没有高清原图');
  return t('高清原图（{size}）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试', { size: formatFileSize(info.hdBytes) });
}

/** The same, said beside the button, where a tooltip is out of reach (a touch screen) and while the pointer rests
 * elsewhere: only the states that go on by themselves or that asking again helps. */
function originalState(state: State): string | null {
  if (state.status === 'downloading') return t('正在下载原图');
  if (state.status === 'failed') return t('没能让微信下载原图');
  if (state.status === 'waiting') return t('微信还没有下载原图');
  return null;
}

/**
 * `onToggle(true)` shows the original instead of the image opened, `onToggle(false)` the image again (#338).
 * `canDownload`: this server has the image auto-download hook, so WeChat can be asked for the original (#766).
 */
export function OriginalButton({ hd, canDownload, onToggle }: { hd: PreviewHd; canDownload: boolean; onToggle(show: boolean): void }) {
  const [state, setState] = useState<State>({ status: 'loading' });
  // pressing again after a failure asks the server again, which runs the whole effect anew
  const [attempt, setAttempt] = useState(0);
  // the answer arrives after the preview may have been drawn again: always the last one shows the original
  const show = useRef(onToggle);
  useEffect(() => { show.current = onToggle; }, [onToggle]);
  useEffect(() => {
    const controller = new AbortController();
    let timer = 0;
    setState({ status: 'loading' });
    // waits for the file WeChat downloads at its next download, and shows it once it is here
    const wait = (tries: number, info: ImageInfo) => {
      if (tries >= POLL_TRIES) { setState({ status: 'waiting', info }); return; }
      timer = window.setTimeout(() => {
        getImageInfo(hd.sessionId, hd.localId, hd.createTime, controller.signal).then(
          next => {
            if (controller.signal.aborted) return;
            if (next.hd) { setState({ status: 'ready', info: next }); show.current(true); }
            else wait(tries + 1, next);
          },
          () => { if (!controller.signal.aborted) wait(tries + 1, info); },
        );
      }, POLL_MS);
    };
    getImageInfo(hd.sessionId, hd.localId, hd.createTime, controller.signal).then(
      info => {
        if (controller.signal.aborted) return;
        // a retry that finds the file here already has what it was pressed for
        if (info.hd) { setState({ status: 'ready', info }); if (attempt > 0) show.current(true); return; }
        // only what the message says has an original is asked for
        if (!canDownload || !info.hdBytes) { setState({ status: 'ready', info }); return; }
        setState({ status: 'downloading', info });
        requestOriginal(hd.sessionId, hd.localId, hd.createTime, controller.signal).then(
          () => { if (!controller.signal.aborted) wait(0, info); },
          () => { if (!controller.signal.aborted) setState({ status: 'failed', info }); },
        );
      },
      () => { if (!controller.signal.aborted) setState({ status: 'error' }); },
    );
    return () => { controller.abort(); window.clearTimeout(timer); };
  }, [hd.src, hd.sessionId, hd.localId, hd.createTime, canDownload, attempt]);

  const size = state.status === 'ready' && state.info.hd && state.info.hdBytes ? ' ' + formatFileSize(state.info.hdBytes) : '';
  // with no image of its own (`.dat`) on this computer, the preview opened on the original already: nothing to go back to
  if (!hd.shown && state.status === 'ready' && state.info.hd && !state.info.normal) {
    return <span className="original-button shown" title={t('正在显示高清原图')}><FrameCorners size={16} />{t('原图')}</span>;
  }
  if (hd.shown) {
    const title = hd.loading ? t('正在载入高清原图，点此换回点开时的图片') : t('正在显示高清原图，点此换回点开时的图片');
    return <button type="button" className="original-button shown" aria-pressed="true" title={title} aria-label={title} onClick={() => onToggle(false)}>
      {hd.loading ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}{t('原图')}<small className="num">{size}</small>
    </button>;
  }
  const hint = originalHint(state);
  const asked = again(state);
  // a request that failed or that WeChat did not answer in a minute is asked again by pressing it
  const blocked = !asked && hint !== null;
  const spinning = state.status === 'loading' || state.status === 'downloading';
  const said = originalState(state);
  // aria-disabled, not disabled: a disabled button takes no pointer, so its hint would not show
  return <>
    <button type="button" className="original-button" aria-pressed={blocked || asked ? undefined : 'false'} aria-disabled={blocked ? 'true' : undefined}
      title={hint ?? t('查看高清原图')} aria-label={hint ? (asked ? hint : t('查看原图：{hint}', { hint })) : t('查看高清原图') + size}
      onClick={() => { if (asked) setAttempt(n => n + 1); else if (!blocked) onToggle(true); }}>
      {spinning ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}{t('原图')}<small className="num">{size}</small>
    </button>
    {said && <span className="original-state" aria-hidden="true">{said}</span>}
  </>;
}
