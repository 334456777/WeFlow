// 预览对话框显示的图片或视频，加载失败时说明无法显示的原因。
import { useEffect, useState } from 'react';
import type { CSSProperties, Ref } from 'react';
import { Warning } from '@phosphor-icons/react';
import { reasonOf, type PreviewThumb } from '../MessageMedia';
import { ZoomImage, type ZoomControls } from './ZoomImage';
import { t } from '../i18n';

/** 这种形状的图片在预览里的大小：在预览的空间限制内尽量宽；加载期间用缩略图作背景。 */
export function previewSize(thumb: PreviewThumb): CSSProperties {
  return {
    // 预览的栏、边角和外边距占的空间：见 shell/PreviewDialog.css 的 `.preview-dialog`
    width: 'min(960px, 100vw - var(--preview-room-x), (100dvh - var(--preview-room-y)) * ' + thumb.aspect.toFixed(4) + ')',
    aspectRatio: String(thumb.aspect),
    background: 'url(' + JSON.stringify(thumb.src) + ') center / contain no-repeat',
  };
}

function MediaError({ src, what }: { src: string; what: string }) {
  const [reason, setReason] = useState('');
  useEffect(() => { let live = true; reasonOf(src).then(r => { if (live) setReason(r); }); return () => { live = false; }; }, [src]);
  return <div className="file-preview"><Warning size={40} /><h3>{t('{what}无法显示', { what })}</h3><p>{reason || t('正在查找原因…')}</p></div>;
}

/** 预览里可缩放的图片；加载失败时说明无法显示的原因。 */
export function PreviewImage({ src, title, controls, thumb, onSettled }: { src: string; title: string; controls?: Ref<ZoomControls>; thumb?: PreviewThumb; onSettled?(): void }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what={t('图片')} />
    : <ZoomImage ref={controls} src={src} alt={title} onLoad={onSettled} onError={() => { setFailed(true); onSettled?.(); }} sized={thumb && previewSize(thumb)} />;
}

/** 预览里的视频；加载失败时说明无法显示的原因。 */
export function PreviewVideo({ src, poster, thumb }: { src: string; poster?: string; thumb?: PreviewThumb }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what={t('视频')} />
    : <video className="preview-video" src={src} poster={poster} controls autoPlay playsInline onError={() => setFailed(true)} style={thumb && previewSize(thumb)} />;
}
