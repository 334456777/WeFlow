// The picture or video the preview dialog shows, and why it cannot be shown when it fails to load.
import { useEffect, useState } from 'react';
import type { CSSProperties, Ref } from 'react';
import { Warning } from '@phosphor-icons/react';
import { reasonOf, type PreviewThumb } from '../MessageMedia';
import { ZoomImage, type ZoomControls } from './ZoomImage';
import { t } from '../i18n';

/** The size of a picture of that shape in the preview: as wide as fits, as the preview's own rules allow. */
export function previewSize(thumb: PreviewThumb): CSSProperties {
  return {
    // the room the preview's bar, corners and margins take: shell/PreviewDialog.css, on .preview-dialog
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

/** The image of the preview, zoomable; why it cannot be shown when it fails to load. */
export function PreviewImage({ src, title, controls, thumb, onSettled }: { src: string; title: string; controls?: Ref<ZoomControls>; thumb?: PreviewThumb; onSettled?(): void }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what={t('图片')} />
    : <ZoomImage ref={controls} src={src} alt={title} onLoad={onSettled} onError={() => { setFailed(true); onSettled?.(); }} sized={thumb && previewSize(thumb)} />;
}

/** The video of the preview; why it cannot be shown when it fails to load. */
export function PreviewVideo({ src, poster, thumb }: { src: string; poster?: string; thumb?: PreviewThumb }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what={t('视频')} />
    : <video className="preview-video" src={src} poster={poster} controls autoPlay playsInline onError={() => setFailed(true)} style={thumb && previewSize(thumb)} />;
}
