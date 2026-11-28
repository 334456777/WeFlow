// 在任何页面打开的图片或视频预览（#194）：从打开它的缩略图里飞出、关闭时飞回，
// 可以在一个会话的图片和视频之间翻看、缩放、显示原图（#338）。
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { flushSync } from 'react-dom';
import { ArrowSquareOut, CaretLeft, CaretRight, X } from '@phosphor-icons/react';
import { PreviewImage, PreviewVideo } from './PreviewMedia';
import type { Preview } from '../MessageMedia';
import { OriginalButton } from './OriginalButton';
import { PreviewDownload } from './PreviewDownload';
import { canFly, fly, frameFlight, inSight, thumbnailOf } from './previewMotion';
import type { ZoomControls } from './ZoomImage';
import { prefersReducedMotion, usePresence } from '../motion';
import { t } from '../i18n';
import './PreviewDialog.css';

/** `usePreview` 返回的内容：打开预览的函数，以及要放进页面的对话框。 */
export interface PreviewControl {
  open(preview: Preview): void;
  /** 对话框，在页面里放一次。 */
  dialog: ReactElement;
}

/** 整个网页共用的预览对话框；`reducedMotion` 时打开和关闭都不播放动画。 */
export function usePreview(reducedMotion: boolean): PreviewControl {
  const [preview, setPreview] = useState<Preview | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const shownPreview = usePresence(preview);
  const zoom = useRef<ZoomControls>(null);
  // 会话里的前一个或后一个图片、视频（#194），并提前读取前后各两个
  const gallery = preview?.gallery;
  const stepPreview = (by: number) => {
    if (!gallery) return;
    const index = gallery.index + by;
    const next = gallery.items[index];
    if (next) setPreview({ ...next, gallery: { items: gallery.items, index } });
  };
  useEffect(() => {
    if (!gallery) return;
    for (const item of gallery.items.slice(Math.max(0, gallery.index - 2), gallery.index + 3)) {
      if (item.type === 'image') new Image().src = item.src;
      else if (item.type === 'video' && item.poster) new Image().src = item.poster;
    }
  }, [gallery]);
  const previewKeys = (event: { key: string; target: EventTarget; preventDefault(): void }) => {
    // 视频自己的按键用来快进快退和调音量
    if (event.target instanceof HTMLVideoElement) return;
    const act: Record<string, () => void> = {
      ArrowLeft: () => stepPreview(-1), ArrowRight: () => stepPreview(1),
      '+': () => zoom.current?.zoomBy(1.25), '=': () => zoom.current?.zoomBy(1.25), '-': () => zoom.current?.zoomBy(0.8), '0': () => zoom.current?.reset(),
    };
    const run = act[event.key];
    if (run) { event.preventDefault(); run(); }
  };

  // 对话框在退场动画播放完之前一直留在顶层。预览渲染时就打开对话框，
  // 从缩略图飞进来的图片（#194）因此能找到打开的对话框；飞回去的图片已经自己关掉了它。
  useLayoutEffect(() => {
    const dialog = dialogRef.current;
    if (shownPreview && !shownPreview.closing && dialog && !dialog.open) dialog.showModal?.();
    else if (!shownPreview && dialog?.open) dialog.close();
  }, [shownPreview]);

  // 预览里的图片或视频从打开它的缩略图（聚焦的或最后按下的那个）飞出、关闭时飞回（#194）。只有预览显示的仍是同一个、
  // 且缩略图在屏幕上时才飞回。
  const pressed = useRef<Element | null>(null);
  useEffect(() => {
    const down = (event: PointerEvent) => { pressed.current = event.target as Element; };
    window.addEventListener('pointerdown', down, true);
    return () => window.removeEventListener('pointerdown', down, true);
  }, []);
  // 打开预览的缩略图，以及当时显示的图片：点开时的图片，或者之后换成的原图
  const opened = useRef<{ thumb: HTMLImageElement; src: string; hd?: string } | null>(null);
  const previewMedia = () => dialogRef.current?.querySelector<HTMLElement>('.zoom-frame img, .preview-video');
  const open = (next: Preview) => {
    const thumb = thumbnailOf(document.activeElement, pressed.current);
    if (!thumb) { opened.current = null; setPreview(next); return; }
    const shown: Preview = { ...next, thumb: { src: thumb.currentSrc || thumb.src, aspect: thumb.naturalWidth / thumb.naturalHeight } };
    opened.current = { thumb, src: next.src, hd: next.type === 'image' ? next.hd?.src : undefined };
    if (!canFly(prefersReducedMotion(reducedMotion))) { setPreview(shown); return; }
    // 对话框打开期间关掉它自己的淡入和放大：中途重新打开会让它们从头播放一遍
    dialogRef.current?.setAttribute('data-flown', '');
    const from = thumb.getBoundingClientRect();
    let done = () => {};
    const frame = () => dialogRef.current?.querySelector<HTMLElement>('.preview-content');
    void fly(thumb, () => flushSync(() => setPreview(shown)), previewMedia, media => {
      const f = frame();
      if (f) done = frameFlight('open', f, media, from);
    }).then(() => done());
  };
  // 在同一个位置换成原图（#194），或者换回点开时的图片（#338）：新的到达之前，原来显示的垫在下面
  const toggleOriginal = (show: boolean) => {
    if (!preview || preview.type !== 'image' || !preview.hd) return;
    const img = previewMedia();
    const under = img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0
      ? { src: img.currentSrc || img.src, aspect: preview.thumb?.aspect ?? img.naturalWidth / img.naturalHeight } : preview.thumb;
    if (show) setPreview({ ...preview, src: preview.hd.src, thumb: under, hd: { ...preview.hd, shown: true, loading: true, opened: preview.src } });
    else if (preview.hd.opened) setPreview({ ...preview, src: preview.hd.opened, thumb: under, hd: { ...preview.hd, shown: false, loading: false } });
  };
  // 原图到了（或者失败了）：「原图」按钮停止转圈
  const originalSettled = () => setPreview(p => (p?.type === 'image' && p.hd?.loading ? { ...p, hd: { ...p.hd, loading: false } } : p));
  const closePreview = () => {
    const back = opened.current;
    opened.current = null;
    const media = previewMedia();
    const dialog = dialogRef.current;
    const same = preview && (preview.src === back?.src || preview.src === back?.hd);
    if (back && media && same && inSight(back.thumb) && canFly(prefersReducedMotion(reducedMotion))) {
      dialog?.setAttribute('data-flown', '');
      const frame = dialog?.querySelector<HTMLElement>('.preview-content');
      const done = frame ? frameFlight('close', frame, media, back.thumb.getBoundingClientRect()) : () => {};
      // 飞回期间又打开了图片（#844）时，那张图片是飞进来的，保留这个标记
      void fly(media, () => { flushSync(() => setPreview(null)); dialog?.close(); }, () => back.thumb).then(landed => {
        done();
        if (landed) dialog?.removeAttribute('data-flown');
      });
    } else {
      dialog?.removeAttribute('data-flown');
      setPreview(null);
    }
  };

  // 点背景关闭预览。框还在飞进来时点在框上，点击也会落到对话框上（框到位之前不接收点击，#844），这种点击什么都不做。
  const onBackdrop = (event: { target: EventTarget; currentTarget: Element; clientX: number; clientY: number }) => {
    if (event.target !== event.currentTarget) return false;
    const box = event.currentTarget.querySelector('.preview-content')?.getBoundingClientRect();
    return !box || event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom;
  };

  const dialog = <dialog className="preview-dialog" ref={dialogRef} aria-label={shownPreview?.value.title || t('媒体预览')} data-state={shownPreview?.closing ? 'closing' : 'open'}
    onCancel={event => { event.preventDefault(); closePreview(); }} onClick={event => { if (onBackdrop(event)) closePreview(); }} onKeyDown={previewKeys}>
    {shownPreview && (() => {
      const shown = shownPreview.value;
      // 预览显示的内容，放开之后的淡出期间也是它（#340）
      const steps = shown.gallery;
      return <div className="preview-content"><header><strong>{shown.title}{steps && <small className="num"> {steps.index + 1} / {steps.items.length}</small>}</strong>
        <span className="preview-tools">
          {shown.locate && <button type="button" className="icon-button" aria-label={t('在会话中定位')} title={t('在会话中定位')}
            onClick={() => { const locate = shown.locate; dialogRef.current?.removeAttribute('data-flown'); opened.current = null; setPreview(null); locate?.(); }}><ArrowSquareOut size={18} /></button>}
          <button type="button" className="icon-button" aria-label={t('关闭预览')} onClick={closePreview}><X size={18} /></button></span></header>
        {/* 图片放在框里的凹陷中（#341） */}
        <div className="preview-stage">{shown.type === 'image'
          ? <PreviewImage key={shown.src} src={shown.src} title={shown.title} controls={zoom} thumb={shown.thumb}
            onSettled={shown.hd?.loading ? originalSettled : undefined} />
          : <PreviewVideo key={shown.src} src={shown.src} poster={shown.poster} thumb={shown.thumb} />}</div>
        <footer className="preview-foot">
          <span className="preview-foot-side">{shown.type === 'image' && shown.hd && <OriginalButton key={shown.hd.src} hd={shown.hd} onToggle={toggleOriginal} />}</span>
          {steps && <span className="preview-steps">
            <button type="button" className="preview-step as-control press-on-hover" aria-label={t('上一个')} title={t('上一个（←）')} disabled={steps.index === 0} onClick={() => stepPreview(-1)}><CaretLeft size={16} /></button>
            <button type="button" className="preview-step as-control press-on-hover" aria-label={t('下一个')} title={t('下一个（→）')} disabled={steps.index === steps.items.length - 1} onClick={() => stepPreview(1)}><CaretRight size={16} /></button>
          </span>}
          <span className="preview-foot-side end">{(shown.type === 'video' || /\/api\/chat\/image-data/.test(shown.src)) &&
            <PreviewDownload key={shown.src} src={shown.src} kind={shown.type} original={shown.type === 'image' && Boolean(shown.hd?.shown)} />}</span>
        </footer>
      </div>;
    })()}
  </dialog>;
  return { open, dialog };
}
