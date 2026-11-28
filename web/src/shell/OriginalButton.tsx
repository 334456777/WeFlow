// 预览左下角的「原图」按钮（#194、#341）。一张图片微信最多保存三个文件：聊天里显示的缩略图、点开时显示的图片
// （预览显示它），以及点「查看原图」时下载的原图。`chat image-info` 说原图在这台电脑上时，按钮切换到原图，
// 再按一次换回点开时的图片（#338）；微信下载原图之前按钮不能按，指针停在上面时说明原因。
import { useEffect, useState } from 'react';
import { CircleNotch, FrameCorners } from '@phosphor-icons/react';
import { getImageInfo, type ImageInfo } from '../api/messages';
import { formatFileSize } from '../format';
import type { PreviewHd } from '../MessageMedia';
import { t } from '../i18n';

type State = { status: 'loading' } | { status: 'ready'; info: ImageInfo } | { status: 'error' };

/** 按钮不能按时的说明；能按时为 null。 */
export function originalHint(state: State): string | null {
  if (state.status === 'loading') return t('正在查看这台电脑上有没有高清原图');
  if (state.status === 'error') return t('没能查到这张图片有没有高清原图');
  const { info } = state;
  if (info.hd) return null;
  if (!info.hdBytes) return t('这张图片没有高清原图');
  return t('高清原图（{size}）还没有下载到这台电脑：在微信里打开这张图片，点「查看原图」后再试', { size: formatFileSize(info.hdBytes) });
}

/** 「原图」按钮：`onToggle(true)` 用原图代替点开时的图片，`onToggle(false)` 换回点开时的图片（#338）。 */
export function OriginalButton({ hd, onToggle }: { hd: PreviewHd; onToggle(show: boolean): void }) {
  const [state, setState] = useState<State>({ status: 'loading' });
  // 每张图片只读一次：显示期间不论显示的是哪一个文件，它有哪些文件在本机都不会变
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
  // 本机没有点开时的那张图片（`.dat`）时，预览一开始显示的就是原图，没有可以换回的
  if (!hd.shown && state.status === 'ready' && state.info.hd && !state.info.normal) {
    return <span className="original-button as-control press-on-hover shown" title={t('正在显示高清原图')}><FrameCorners size={16} />{t('原图')}</span>;
  }
  if (hd.shown) {
    const title = hd.loading ? t('正在载入高清原图，点此换回点开时的图片') : t('正在显示高清原图，点此换回点开时的图片');
    return <button type="button" className="original-button as-control press-on-hover shown" aria-pressed="true" title={title} aria-label={title} onClick={() => onToggle(false)}>
      {hd.loading ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}{t('原图')}<small className="num">{size}</small>
    </button>;
  }
  const hint = originalHint(state);
  // 用 `aria-disabled` 而不是 `disabled`：`disabled` 的按钮不响应指针，说明就显示不出来
  return <button type="button" className="original-button as-control press-on-hover" aria-pressed={hint ? undefined : 'false'} aria-disabled={hint ? 'true' : undefined} title={hint ?? t('查看高清原图')}
    aria-label={hint ? t('查看原图：{hint}', { hint }) : t('查看高清原图') + size} onClick={() => { if (!hint) onToggle(true); }}>
    {state.status === 'loading' ? <CircleNotch size={16} className="spin" /> : <FrameCorners size={16} />}{t('原图')}<small className="num">{size}</small>
  </button>;
}
