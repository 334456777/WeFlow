// One message of the chat window (#198), drawn by its kind on the surfaces of #220: received bubbles raised,
// sent ones inset; cards (links, files, records…) are bubbles too.
import { useEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, Ref } from 'react';
import {
  Article, ChatsCircle, CurrencyCny, File, FilePdf, FileText, Gift, Image as ImageIcon, MapPin, Pause, Phone, Play,
  UserSquare, VideoCamera, Warning,
} from '@phosphor-icons/react';
import { mediaUrl, type ParsedMessage } from '../api/messages';
import { appLabel, formatDuration, formatFileSize, kindOf, withoutLabel } from './messages';
import { ZoomImage, type ZoomControls } from './ZoomImage';

export type Preview =
  | { type: 'image'; src: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb; hd?: PreviewHd }
  | { type: 'video'; src: string; poster?: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb }
  | { type: 'file'; title: string };

/** The original WeChat downloads on View original (`_h.dat`) of a picture the preview shows as opened (`.dat`), to
 * show instead once it is on this computer (#194); `shown` once asked for, `loading` until it has come, `opened` the
 * image it took the place of, to go back to (#338). */
export interface PreviewHd { src: string; sessionId: string; localId: number; createTime: number; shown?: boolean; loading?: boolean; opened?: string }

/** A picture message as the preview opens it: the image WeChat shows when it is opened, with its original beside. */
export function imagePreview(sessionId: string, message: ParsedMessage, title: string): Extract<Preview, { type: 'image' }> {
  return {
    type: 'image', src: mediaUrl('image-data', sessionId, message, { variant: 'normal' }), title,
    hd: { src: mediaUrl('image-data', sessionId, message, { variant: 'hd' }), sessionId, localId: message.localId, createTime: message.createTime },
  };
}

/** The thumbnail a preview opened from (#194): the preview takes its shape at once and shows it until the picture
 * comes, so the picture that flies in from it has a place to land and does not move again when it loads. */
export interface PreviewThumb { src: string; aspect: number }

/** The size of a picture of that shape in the preview: as wide as fits, as the preview's own rules allow. */
export function previewSize(thumb: PreviewThumb): CSSProperties {
  return {
    // the room the preview's bar, corners and margins take: app.css, on .preview-dialog
    width: 'min(960px, 100vw - var(--preview-room-x), (100dvh - var(--preview-room-y)) * ' + thumb.aspect.toFixed(4) + ')',
    aspectRatio: String(thumb.aspect),
    background: 'url(' + JSON.stringify(thumb.src) + ') center / contain no-repeat',
  };
}

/** The pictures and videos of the conversation read so far, the one shown at `index` (#194). */
export interface Gallery { items: Preview[]; index: number }

/** What a picture or video message opens in the preview; null for other messages. */
export function mediaPreview(sessionId: string, message: ParsedMessage): Preview | null {
  const kind = kindOf(message);
  if (kind === 'image') return imagePreview(sessionId, message, '图片');
  if (kind === 'video') return { type: 'video', src: mediaUrl('video-data', sessionId, message), poster: mediaUrl('video-data', sessionId, message, { thumb: true }), title: '视频' };
  return null;
}

/** The preview of `message` with the pictures and videos among `messages` to go through. */
export function withGallery(sessionId: string, messages: readonly ParsedMessage[], message: ParsedMessage): Preview | null {
  const own = mediaPreview(sessionId, message);
  if (!own || own.type === 'file') return own;
  const media = messages.filter(m => mediaPreview(sessionId, m));
  const index = media.findIndex(m => m._cursor === message._cursor);
  if (index < 0 || media.length < 2) return own;
  return { ...own, gallery: { items: media.map(m => mediaPreview(sessionId, m)!), index } };
}

/** Pictures from WeChat's servers: no referrer, loaded when near the screen. */
function RemoteImage({ src, className, alt = '' }: { src: string; className?: string; alt?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className={'media-missing ' + (className ?? '')}><ImageIcon size={20} /></span>;
  return <img className={className} src={src} alt={alt} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} />;
}

/** One voice plays at a time across the page. */
let playing: { audio: HTMLAudioElement; stop(): void } | null = null;

export function Voice({ sessionId, message, className = 'message-bubble voice-bubble' }: { sessionId: string; message: ParsedMessage; className?: string }) {
  const [state, setState] = useState<'idle' | 'loading' | 'playing' | 'error'>('idle');
  const audio = useRef<HTMLAudioElement | null>(null);
  useEffect(() => () => { audio.current?.pause(); }, []);
  const seconds = message.voiceDurationSeconds ?? 0;
  const toggle = () => {
    if (state === 'playing' || state === 'loading') { audio.current?.pause(); setState('idle'); return; }
    playing?.stop();
    const sound = audio.current ?? new Audio(mediaUrl('voice-data', sessionId, message, {
      'server-id': message.serverIdRaw, sender: message.senderUsername ?? undefined,
    }));
    audio.current = sound;
    sound.onended = () => setState('idle');
    sound.onerror = () => setState('error');
    sound.onplaying = () => setState('playing');
    playing = { audio: sound, stop: () => { sound.pause(); setState('idle'); } };
    setState('loading');
    sound.currentTime = 0;
    sound.play().catch(() => setState('error'));
  };
  // Longer voices make a longer bubble, as in WeChat.
  const width = Math.min(220, 72 + seconds * 4);
  return <button type="button" className={className + ' ' + state} style={className.includes('voice-bubble') ? { width } : undefined} onClick={toggle}
    aria-label={(state === 'playing' ? '暂停' : '播放') + '语音，' + formatDuration(seconds)}>
    {state === 'error' ? <Warning size={16} /> : state === 'playing' ? <Pause size={16} weight="fill" /> : <Play size={16} weight="fill" />}
    <span className="voice-wave" aria-hidden="true" />
    <span className="num">{state === 'error' ? '无法播放' : formatDuration(seconds)}</span>
  </button>;
}

function fileIcon(ext = '') {
  if (/^pdf$/i.test(ext)) return <FilePdf size={28} />;
  if (/^(txt|md|docx?|xlsx?|pptx?|csv)$/i.test(ext)) return <FileText size={28} />;
  return <File size={28} />;
}

const safeUrl = (url?: string) => (url && /^https?:\/\//i.test(url) ? url : undefined);

/** A card bubble that opens its link in a new tab when it has one. */
function Card({ href, className, children, label }: { href?: string; className: string; children: ReactNode; label?: string }) {
  const url = safeUrl(href);
  return url
    ? <a className={'message-bubble card-bubble ' + className} href={url} target="_blank" rel="noopener noreferrer" aria-label={label}>{children}</a>
    : <div className={'message-bubble card-bubble ' + className}>{children}</div>;
}

function Record({ message }: { message: ParsedMessage }) {
  const [open, setOpen] = useState(false);
  const lines = (message.appMsgDesc ?? '').split('\n').filter(Boolean);
  return <div className="message-bubble card-bubble record-bubble">
    <strong className="card-bubble-title"><ChatsCircle size={16} />{message.chatRecordTitle || message.linkTitle || '聊天记录'}</strong>
    <span className={'record-lines' + (open ? ' open' : '')}>{lines.map((line, i) => <span key={i}>{line}</span>)}</span>
    {lines.length > 3 && <button type="button" className="text-button" aria-expanded={open} onClick={() => setOpen(v => !v)}>{open ? '收起' : '展开'}</button>}
    <small className="card-bubble-foot">聊天记录</small>
  </div>;
}

export function MessageView({ sessionId, message, onPreview }: {
  sessionId: string; message: ParsedMessage; onPreview(preview: Preview): void;
}) {
  const text = message.parsedContent;
  switch (kindOf(message)) {
    case 'image': {
      const thumb = mediaUrl('image-data', sessionId, message, { thumb: true });
      return <button type="button" className="photo-button" aria-label="预览图片"
        onClick={() => onPreview(mediaPreview(sessionId, message)!)}>
        <MediaThumb src={thumb} label="图片" />
      </button>;
    }
    case 'video': {
      const poster = mediaUrl('video-data', sessionId, message, { thumb: true });
      return <button type="button" className="photo-button video-button" aria-label="播放视频"
        onClick={() => onPreview(mediaPreview(sessionId, message)!)}>
        <MediaThumb src={poster} label="视频" /><span className="play-badge" aria-hidden="true"><Play size={20} weight="fill" /></span>
      </button>;
    }
    case 'emoji':
      return message.emojiCdnUrl
        ? <span className="emoji-sticker"><RemoteImage src={message.emojiCdnUrl} alt="动画表情" /></span>
        : <div className="message-bubble">{text || '[动画表情]'}</div>;
    case 'voice':
      return <Voice sessionId={sessionId} message={message} />;
    case 'file':
      return <div className="message-bubble file-bubble">{fileIcon(message.fileExt)}<span><span className="clamp-2">{message.fileName || withoutLabel(text)}</span><small>{formatFileSize(message.fileSize) || '文件'}</small></span></div>;
    case 'quote':
      return <div className="message-bubble quote-bubble"><span>{text}</span>
        {message.quotedContent && <span className="quote-block clamp-2">{message.quotedSender ? message.quotedSender + '：' : ''}{message.quotedContent}</span>}
      </div>;
    case 'link':
      return <Card href={message.linkUrl} className="link-bubble" label={message.linkTitle}>
        <strong className="card-bubble-title clamp-2">{message.linkTitle || withoutLabel(text)}</strong>
        <span className="card-bubble-body">
          {message.appMsgDesc && <span className="clamp-3">{message.appMsgDesc}</span>}
          {(message.linkThumb || message.appMsgThumbUrl) && <RemoteImage className="card-thumb" src={(message.linkThumb || message.appMsgThumbUrl)!} />}
        </span>
        <small className="card-bubble-foot">{message.appMsgKind === 'official-link' ? <Article size={12} /> : null}{appLabel(message)}</small>
      </Card>;
    case 'finder':
      return <Card href={message.linkUrl} className="finder-bubble" label={message.linkTitle}>
        {message.finderCoverUrl && <span className="finder-cover"><RemoteImage src={message.finderCoverUrl} /><span className="play-badge" aria-hidden="true"><Play size={18} weight="fill" /></span></span>}
        <strong className="card-bubble-title clamp-2">{message.linkTitle || withoutLabel(text)}</strong>
        <small className="card-bubble-foot"><VideoCamera size={12} />视频号{message.finderNickname ? ' · ' + message.finderNickname : ''}</small>
      </Card>;
    case 'record':
      return <Record message={message} />;
    case 'contact':
      return <div className="message-bubble card-bubble contact-bubble">
        <span className="contact-line">{message.cardAvatarUrl ? <RemoteImage className="avatar" src={message.cardAvatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true">{Array.from(message.cardNickname ?? '?')[0]}</span>}<strong className="clamp-1">{message.cardNickname || withoutLabel(text)}</strong></span>
        <small className="card-bubble-foot"><UserSquare size={12} />个人名片</small>
      </div>;
    case 'location':
      return <div className="message-bubble card-bubble location-bubble">
        <strong className="card-bubble-title"><MapPin size={16} />{message.locationPoiname || withoutLabel(text) || '位置'}</strong>
        {message.locationLabel && <span className="clamp-2">{message.locationLabel}</span>}
      </div>;
    case 'money':
      return <Card href={message.linkUrl} className={'money-bubble ' + (message.appMsgKind ?? '')}>
        <strong className="card-bubble-title">{message.appMsgKind === 'transfer' ? <CurrencyCny size={18} /> : <Gift size={18} />}<span className="clamp-2">{message.linkTitle || withoutLabel(text)}</span></strong>
        {message.appMsgDesc && <span className="clamp-2">{message.appMsgDesc}</span>}
        <small className="card-bubble-foot">{appLabel(message)}</small>
      </Card>;
    case 'call':
      return <div className="message-bubble call-bubble"><Phone size={16} />{withoutLabel(text) || text}</div>;
    case 'system':
      return <p className="system-message">{text}</p>;
    default:
      return <div className="message-bubble">{text}</div>;
  }
}

/** Why a media address failed: the server answers its error as JSON (a video not downloaded yet, a missing ffmpeg…). */
export async function reasonOf(src: string): Promise<string> {
  try {
    const response = await fetch(src, { headers: { Range: 'bytes=0-0' } });
    const doc = await response.json();
    return doc?.error?.message || '无法读取（HTTP ' + response.status + '）';
  } catch {
    return '无法读取';
  }
}

function MediaError({ src, what }: { src: string; what: string }) {
  const [reason, setReason] = useState('');
  useEffect(() => { let live = true; reasonOf(src).then(r => { if (live) setReason(r); }); return () => { live = false; }; }, [src]);
  return <div className="file-preview"><Warning size={40} /><h3>{what}无法显示</h3><p>{reason || '正在查找原因…'}</p></div>;
}

export function PreviewImage({ src, title, controls, thumb, onSettled }: { src: string; title: string; controls?: Ref<ZoomControls>; thumb?: PreviewThumb; onSettled?(): void }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what="图片" />
    : <ZoomImage ref={controls} src={src} alt={title} onLoad={onSettled} onError={() => { setFailed(true); onSettled?.(); }} sized={thumb && previewSize(thumb)} />;
}

export function PreviewVideo({ src, poster, thumb }: { src: string; poster?: string; thumb?: PreviewThumb }) {
  const [failed, setFailed] = useState(false);
  return failed
    ? <MediaError src={src} what="视频" />
    : <video className="preview-video" src={src} poster={poster} controls autoPlay playsInline onError={() => setFailed(true)} style={thumb && previewSize(thumb)} />;
}

/** A thumbnail from the server; when it cannot be decoded, a placeholder that says so instead of a broken image. */
function MediaThumb({ src, label }: { src: string; label: string }) {
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading');
  return state === 'failed'
    ? <span className="media-missing"><Warning size={20} /><small>{label}无法显示</small></span>
    : <img className={'message-thumb' + (state === 'loading' ? ' loading' : '')} src={src} alt={label} loading="lazy" decoding="async"
      onLoad={() => setState('ready')} onError={() => setState('failed')} />;
}
