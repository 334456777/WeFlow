// 「原图」 in the preview's lower left corner (#194, #341): WeChat keeps up to three files of a picture: the thumbnail a
// chat shows, the image shown when it is opened, which the preview shows, and the original it downloads on View
// original. The button shows the original once `chat image-info` says it is on this computer, and pressed again the
// image opened (#338); until WeChat has downloaded the original it cannot be pressed and says why when the pointer
// rests on it.
import { useEffect, useState } from 'react';
import { CircleNotch, FrameCorners } from '@phosphor-icons/react';
import { getImageInfo, type ImageInfo } from '../api/messages';
import { formatFileSize } from './messages';
import type { PreviewHd } from './MessageView';

type State = { status: 'loading' } | { status: 'ready'; info: ImageInfo } | { status: 'error' };

/** What the button says when it cannot be pressed, or null when it can. */
export function originalHint(state: State): string | null {
  if (state.status === 'loading') return '正在查看这台电脑上有没有高清原图';
  if (state.status === 'error') return '没能查到这张图片有没有高清原图';
  const { info } = state;
  if (info.hd) return null;
  if (!info.hdBytes) return '这张图片没有高清原图';
  return '高清原图（' + formatFileSize(info.hdBytes) + '）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试';
}

/** `onToggle(true)` shows the original instead of the image opened, `onToggle(false)` the image again (#338). */
export function OriginalButton({ hd, onToggle }: { hd: PreviewHd; onToggle(show: boolean): void }) {
  const [state, setState] = useState<State>({ status: 'loading' });
  // read once a picture: which of its files are here does not change while it is shown, either of them
  useEffect(() => {
    const controller = new AbortController();
    setState({ status: 'loading' });
    getImageInfo(hd.sessionId, hd.localId, hd.createTime, controller.signal).then(
      info => { if (!controller.signal.aborted) setState({ status: 'ready', info }); },
      () => { if (!controller.signal.aborted) setState({ status: 'error' }); },
    );
    return () => controller.abort();
  }, [hd.src, hd.sessionId, hd.localId, hd.createTime]);

  const size = state.status === 'ready' && state.info.hd && state.info.hdBytes ? ' ' + formatFileSize(state.info.hdBytes) : '';
  // with no image of its own (`.dat`) on this computer, the preview opened on the original already: nothing to go back to
  if (!hd.shown && state.status === 'ready' && state.info.hd && !state.info.normal) {
    return <span className="original-button shown" title="正在显示高清原图"><FrameCorners size={16} />原图</span>;
  }
  if (hd.shown) {
    const title = hd.loading ? '正在载入高清原图，点此换回点开时的图片' : '正在显示高清原图，点此换回点开时的图片';
    return <button type="button" className="original-button shown" aria-pressed="true" title={title} aria-label={title} onClick={() => onToggle(false)}>
      {hd.loading ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}原图<small className="num">{size}</small>
    </button>;
  }
  const hint = originalHint(state);
  // aria-disabled, not disabled: a disabled button takes no pointer, so its hint would not show
  return <button type="button" className="original-button" aria-pressed={hint ? undefined : 'false'} aria-disabled={hint ? 'true' : undefined} title={hint ?? '查看高清原图'}
    aria-label={hint ? '查看原图：' + hint : '查看高清原图' + size} onClick={() => { if (!hint) onToggle(true); }}>
    {state.status === 'loading' ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}原图<small className="num">{size}</small>
  </button>;
}
