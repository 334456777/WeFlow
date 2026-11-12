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
  /** Asking failed: the server could not leave the request. */
  | { status: 'failed'; info: ImageInfo }
  /** Waited a minute: WeChat has not downloaded it yet. */
  | { status: 'waiting'; info: ImageInfo }
  | { status: 'error' };

/** What the button says when it cannot be pressed, or null when it can. */
export function originalHint(state: State): string | null {
  if (state.status === 'loading') return t('正在查看这台电脑上有没有高清原图');
  if (state.status === 'error') return t('没能查到这张图片有没有高清原图');
  if (state.status === 'downloading') return t('正在下载原图');
  const { info } = state;
  if (info.hd) return null;
  if (!info.hdBytes) return t('这张图片没有高清原图');
  if (state.status === 'failed') return t('没能让微信下载原图');
  if (state.status === 'waiting') return t('微信还没有下载原图：在微信里打开这张图片，点「查看原图」后再试');
  return t('高清原图（{size}）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试', { size: formatFileSize(info.hdBytes) });
}

/**
 * `onToggle(true)` shows the original instead of the image opened, `onToggle(false)` the image again (#338).
 * `canDownload`: this server has the image auto-download hook, so WeChat can be asked for the original (#766).
 */
export function OriginalButton({ hd, canDownload, onToggle }: { hd: PreviewHd; canDownload: boolean; onToggle(show: boolean): void }) {
  const [state, setState] = useState<State>({ status: 'loading' });
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
        // only what the message says has an original is asked for
        if (info.hd || !canDownload || !info.hdBytes) { setState({ status: 'ready', info }); return; }
        setState({ status: 'downloading', info });
        requestOriginal(hd.sessionId, hd.localId, hd.createTime, controller.signal).then(
          () => { if (!controller.signal.aborted) wait(0, info); },
          () => { if (!controller.signal.aborted) setState({ status: 'failed', info }); },
        );
      },
      () => { if (!controller.signal.aborted) setState({ status: 'error' }); },
    );
    return () => { controller.abort(); window.clearTimeout(timer); };
  }, [hd.src, hd.sessionId, hd.localId, hd.createTime, canDownload]);

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
  const spinning = state.status === 'loading' || state.status === 'downloading';
  // aria-disabled, not disabled: a disabled button takes no pointer, so its hint would not show
  return <button type="button" className="original-button" aria-pressed={hint ? undefined : 'false'} aria-disabled={hint ? 'true' : undefined} title={hint ?? t('查看高清原图')}
    aria-label={hint ? t('查看原图：{hint}', { hint }) : t('查看高清原图') + size} onClick={() => { if (!hint) onToggle(true); }}>
    {spinning ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}{t('原图')}<small className="num">{size}</small>
  </button>;
}
