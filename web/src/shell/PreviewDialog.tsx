// The preview of a picture, video or file opened on any page (#194): it flies out of the thumbnail it was opened from
// and back into it, steps through the pictures and videos of a conversation, zooms, shows the original (#338).
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { flushSync } from 'react-dom';
import { ArrowSquareOut, CaretLeft, CaretRight, FilePdf, X } from '@phosphor-icons/react';
import { PreviewImage, PreviewVideo, type Preview } from '../chat/MessageView';
import { OriginalButton } from '../chat/OriginalButton';
import { PreviewDownload } from '../chat/PreviewDownload';
import { canFly, fly, frameFlight, inSight, thumbnailOf } from '../chat/previewMotion';
import type { ZoomControls } from '../chat/ZoomImage';
import { prefersReducedMotion, usePresence } from '../motion';
import { t } from '../i18n';

export interface PreviewControl {
  open(preview: Preview): void;
  /** The dialog, once in the page. */
  dialog: ReactElement;
}

export function usePreview(reducedMotion: boolean): PreviewControl {
  const [preview, setPreview] = useState<Preview | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const shownPreview = usePresence(preview);
  const zoom = useRef<ZoomControls>(null);
  // the picture or video before or after in the conversation (#194), and the two each way read ahead of time
  const gallery = preview && preview.type !== 'file' ? preview.gallery : undefined;
  const stepPreview = (by: number) => {
    if (!gallery) return;
    const index = gallery.index + by;
    const next = gallery.items[index];
    if (next && next.type !== 'file') setPreview({ ...next, gallery: { items: gallery.items, index } });
  };
  useEffect(() => {
    if (!gallery) return;
    for (const item of gallery.items.slice(Math.max(0, gallery.index - 2), gallery.index + 3)) {
      if (item.type === 'image') new Image().src = item.src;
      else if (item.type === 'video' && item.poster) new Image().src = item.poster;
    }
  }, [gallery]);
  const previewKeys = (event: { key: string; target: EventTarget; preventDefault(): void }) => {
    // the video's own keys seek and change the volume
    if (event.target instanceof HTMLVideoElement) return;
    const act: Record<string, () => void> = {
      ArrowLeft: () => stepPreview(-1), ArrowRight: () => stepPreview(1),
      '+': () => zoom.current?.zoomBy(1.25), '=': () => zoom.current?.zoomBy(1.25), '-': () => zoom.current?.zoomBy(0.8), '0': () => zoom.current?.reset(),
    };
    const run = act[event.key];
    if (run) { event.preventDefault(); run(); }
  };

  // The dialog stays in the top layer until its exit animation has played. Opened as the preview is rendered, so that
  // a picture flying in from its thumbnail (#194) finds it open; one that flew back closed it already.
  useLayoutEffect(() => {
    const dialog = dialogRef.current;
    if (shownPreview && !shownPreview.closing && dialog && !dialog.open) dialog.showModal?.();
    else if (!shownPreview && dialog?.open) dialog.close();
  }, [shownPreview]);

  // The picture or video of a preview flies out of the thumbnail it was opened from and back into it (#194): the one
  // focused or last pressed. Back only while the preview shows that same one and its thumbnail is on screen.
  const pressed = useRef<Element | null>(null);
  useEffect(() => {
    const down = (event: PointerEvent) => { pressed.current = event.target as Element; };
    window.addEventListener('pointerdown', down, true);
    return () => window.removeEventListener('pointerdown', down, true);
  }, []);
  // the thumbnail opened from, and the picture it showed then: the image, or its original once shown instead
  const opened = useRef<{ thumb: HTMLImageElement; src: string; hd?: string } | null>(null);
  const previewMedia = () => dialogRef.current?.querySelector<HTMLElement>('.zoom-frame img, .preview-video');
  const open = (next: Preview) => {
    const thumb = next.type === 'file' ? null : thumbnailOf(document.activeElement, pressed.current);
    if (!thumb || next.type === 'file') { opened.current = null; setPreview(next); return; }
    const shown: Preview = { ...next, thumb: { src: thumb.currentSrc || thumb.src, aspect: thumb.naturalWidth / thumb.naturalHeight } };
    opened.current = { thumb, src: next.src, hd: next.type === 'image' ? next.hd?.src : undefined };
    if (!canFly(prefersReducedMotion(reducedMotion))) { setPreview(shown); return; }
    // the dialog's own fade and growth stay off while it is open: switched back on, they would play from the start
    dialogRef.current?.setAttribute('data-flown', '');
    const from = thumb.getBoundingClientRect();
    let done = () => {};
    const frame = () => dialogRef.current?.querySelector<HTMLElement>('.preview-content');
    void fly(thumb, () => flushSync(() => setPreview(shown)), previewMedia, media => {
      const f = frame();
      if (f) done = frameFlight('open', f, media, from);
    }).then(() => done());
  };
  // the original instead of the image opened (#194), or the image opened again (#338), in the same place: the one shown
  // stays under the other until it comes
  const toggleOriginal = (show: boolean) => {
    if (!preview || preview.type !== 'image' || !preview.hd) return;
    const img = previewMedia();
    const under = img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0
      ? { src: img.currentSrc || img.src, aspect: preview.thumb?.aspect ?? img.naturalWidth / img.naturalHeight } : preview.thumb;
    if (show) setPreview({ ...preview, src: preview.hd.src, thumb: under, hd: { ...preview.hd, shown: true, loading: true, opened: preview.src } });
    else if (preview.hd.opened) setPreview({ ...preview, src: preview.hd.opened, thumb: under, hd: { ...preview.hd, shown: false, loading: false } });
  };
  // the original has come (or failed): 「原图」 stops spinning
  const originalSettled = () => setPreview(p => (p?.type === 'image' && p.hd?.loading ? { ...p, hd: { ...p.hd, loading: false } } : p));
  const closePreview = () => {
    const back = opened.current;
    opened.current = null;
    const media = previewMedia();
    const dialog = dialogRef.current;
    const same = preview && preview.type !== 'file' && (preview.src === back?.src || preview.src === back?.hd);
    if (back && media && same && inSight(back.thumb) && canFly(prefersReducedMotion(reducedMotion))) {
      dialog?.setAttribute('data-flown', '');
      const frame = dialog?.querySelector<HTMLElement>('.preview-content');
      const done = frame ? frameFlight('close', frame, media, back.thumb.getBoundingClientRect()) : () => {};
      void fly(media, () => { flushSync(() => setPreview(null)); dialog?.close(); }, () => back.thumb).then(() => { done(); dialog?.removeAttribute('data-flown'); });
    } else {
      dialog?.removeAttribute('data-flown');
      setPreview(null);
    }
  };

  const dialog = <dialog className="preview-dialog" ref={dialogRef} aria-label={shownPreview?.value.title || t('媒体预览')} data-state={shownPreview?.closing ? 'closing' : 'open'}
    onCancel={event => { event.preventDefault(); closePreview(); }} onClick={event => { if (event.target === event.currentTarget) closePreview(); }} onKeyDown={previewKeys}>
    {shownPreview && (() => {
      const shown = shownPreview.value;
      // what the preview shows, also while it fades out after it was let go of (#340)
      const steps = shown.type !== 'file' ? shown.gallery : undefined;
      return <div className="preview-content"><header><strong>{shown.title}{steps && <small className="num"> {steps.index + 1} / {steps.items.length}</small>}</strong>
        <span className="preview-tools">
          {shown.type !== 'file' && shown.locate && <button type="button" className="icon-button" aria-label={t('在会话中定位')} title={t('在会话中定位')}
            onClick={() => { const locate = shown.locate; dialogRef.current?.removeAttribute('data-flown'); opened.current = null; setPreview(null); locate?.(); }}><ArrowSquareOut size={18} /></button>}
          <button type="button" className="icon-button" aria-label={t('关闭预览')} onClick={closePreview}><X size={18} /></button></span></header>
        {shown.type === 'file'
          ? <div className="file-preview"><FilePdf size={56} /><h3>{shown.title}</h3><p>{t('示例文件记录，可查看名称与记录时间。')}</p></div>
          : <>
            {/* the picture sunk in a hollow of the frame (#341) */}
            <div className="preview-stage">{shown.type === 'image'
              ? <PreviewImage key={shown.src} src={shown.src} title={shown.title} controls={zoom} thumb={shown.thumb}
                onSettled={shown.hd?.loading ? originalSettled : undefined} />
              : <PreviewVideo key={shown.src} src={shown.src} poster={shown.poster} thumb={shown.thumb} />}</div>
            <footer className="preview-foot">
              <span className="preview-foot-side">{shown.type === 'image' && shown.hd && <OriginalButton key={shown.hd.src} hd={shown.hd} onToggle={toggleOriginal} />}</span>
              {steps && <span className="preview-steps">
                <button type="button" className="preview-step" aria-label={t('上一个')} title={t('上一个（←）')} disabled={steps.index === 0} onClick={() => stepPreview(-1)}><CaretLeft size={16} /></button>
                <button type="button" className="preview-step" aria-label={t('下一个')} title={t('下一个（→）')} disabled={steps.index === steps.items.length - 1} onClick={() => stepPreview(1)}><CaretRight size={16} /></button>
              </span>}
              <span className="preview-foot-side end">{(shown.type === 'video' || /\/api\/chat\/image-data/.test(shown.src)) &&
                <PreviewDownload key={shown.src} src={shown.src} kind={shown.type} original={shown.type === 'image' && Boolean(shown.hd?.shown)} />}</span>
            </footer>
          </>}
      </div>;
    })()}
  </dialog>;
  return { open, dialog };
}
