// 聊天窗口里的一条消息（#198），按种类画成皮肤的气泡（#828）：收到的和账号所有者自己发的样子不同；
// 卡片（链接、文件、聊天记录……）也是气泡。
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Article, ChatsCircle, CurrencyCny, File, FilePdf, FileText, Gift, Image as ImageIcon, MapPin, Phone, Play,
  UserSquare, VideoCamera, Warning,
} from '@phosphor-icons/react';
import { mediaUrl, type ParsedMessage } from '../api/messages';
import { formatFileSize } from '../format';
import { imagePreview, PlayBadge, videoPlays, Voice, type Preview } from '../MessageMedia';
import { kindOf } from '../messageKind';
import { appLabel, withoutLabel } from './messages';
import { t } from '../i18n';
import { safeUrl } from '../links';

/** 图片或视频消息在预览里打开的内容；其他消息为 null。 */
export function mediaPreview(sessionId: string, message: ParsedMessage): Preview | null {
  const kind = kindOf(message);
  if (kind === 'image') return imagePreview(sessionId, message, t('图片'));
  if (kind === 'video') return { type: 'video', src: mediaUrl('video-data', sessionId, message), poster: mediaUrl('video-data', sessionId, message, { thumb: true }), title: t('视频') };
  return null;
}

/** `message` 的预览，带上 `messages` 中的图片和视频供前后翻看；`message` 不是图片或视频时为 null。 */
export function withGallery(sessionId: string, messages: readonly ParsedMessage[], message: ParsedMessage): Preview | null {
  const own = mediaPreview(sessionId, message);
  if (!own) return null;
  const media = messages.filter(m => mediaPreview(sessionId, m));
  const index = media.findIndex(m => m._cursor === message._cursor);
  if (index < 0 || media.length < 2) return own;
  return { ...own, gallery: { items: media.map(m => mediaPreview(sessionId, m)!), index } };
}

/** 来自微信服务器的图片：不发送 Referer，接近屏幕时才加载，加载失败时显示占位。 */
function RemoteImage({ src, className, alt = '' }: { src: string; className?: string; alt?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className={'media-missing ' + (className ?? '')}><ImageIcon size={20} /></span>;
  return <img className={className} src={src} alt={alt} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} />;
}

function fileIcon(ext = '') {
  if (/^pdf$/i.test(ext)) return <FilePdf size={28} />;
  if (/^(txt|md|docx?|xlsx?|pptx?|csv)$/i.test(ext)) return <FileText size={28} />;
  return <File size={28} />;
}

/** 卡片气泡：有可以打开的链接时，点击后在新标签页打开。 */
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
    <strong className="card-bubble-title"><ChatsCircle size={16} />{message.chatRecordTitle || message.linkTitle || t('聊天记录')}</strong>
    <span className={'record-lines' + (open ? ' open' : '')}>{lines.map((line, i) => <span key={i}>{line}</span>)}</span>
    {lines.length > 3 && <button type="button" className="text-button" aria-expanded={open} onClick={() => setOpen(v => !v)}>{open ? t('收起') : t('展开')}</button>}
    <small className="card-bubble-foot">{t('聊天记录')}</small>
  </div>;
}

/** 按种类画出的一条消息。 */
export function MessageView({ sessionId, message, onPreview }: {
  sessionId: string; message: ParsedMessage; onPreview(preview: Preview): void;
}) {
  const text = message.parsedContent;
  switch (kindOf(message)) {
    case 'image': {
      const thumb = mediaUrl('image-data', sessionId, message, { variant: 'thumb' });
      return <button type="button" className="photo-button" aria-label={t('预览图片')}
        onClick={() => onPreview(mediaPreview(sessionId, message)!)}>
        <MediaThumb src={thumb} label={t('图片')} />
      </button>;
    }
    case 'video': {
      const poster = mediaUrl('video-data', sessionId, message, { thumb: true });
      return <button type="button" className="photo-button video-button" aria-label={t('播放视频')}
        onClick={() => onPreview(mediaPreview(sessionId, message)!)}>
        <MediaThumb src={poster} label={t('视频')} video={mediaUrl('video-data', sessionId, message)} badge={<PlayBadge size={20} />} />
      </button>;
    }
    case 'emoji':
      return message.emojiCdnUrl
        ? <span className="emoji-sticker"><RemoteImage src={message.emojiCdnUrl} alt={t('动画表情')} /></span>
        : <div className="message-bubble">{text || t('[动画表情]')}</div>;
    case 'voice':
      return <Voice sessionId={sessionId} message={message} />;
    case 'file':
      return <div className="message-bubble file-bubble">{fileIcon(message.fileExt)}<span><span className="clamp-2">{message.fileName || withoutLabel(text)}</span><small>{formatFileSize(message.fileSize) || t('文件')}</small></span></div>;
    case 'quote':
      return <div className="message-bubble quote-bubble"><span>{text}</span>
        {message.quotedContent && <span className="quote-block clamp-2">{message.quotedSender ? message.quotedSender + '：' : ''}{message.quotedContent}</span>}
      </div>;
    case 'link':
      return <Card href={message.linkUrl} className="link-bubble" label={message.linkTitle}>
        <strong className="card-bubble-title"><span className="clamp-2">{message.linkTitle || withoutLabel(text)}</span></strong>
        <span className="card-bubble-body">
          {message.appMsgDesc && <span className="clamp-3">{message.appMsgDesc}</span>}
          {(message.linkThumb || message.appMsgThumbUrl) && <RemoteImage className="card-thumb" src={(message.linkThumb || message.appMsgThumbUrl)!} />}
        </span>
        <small className="card-bubble-foot">{message.appMsgKind === 'official-link' ? <Article size={12} /> : null}{appLabel(message)}</small>
      </Card>;
    case 'finder':
      return <Card href={message.linkUrl} className="finder-bubble" label={message.linkTitle}>
        {message.finderCoverUrl && <span className="finder-cover"><RemoteImage src={message.finderCoverUrl} /><span className="play-badge" aria-hidden="true"><Play size={18} weight="fill" /></span></span>}
        <strong className="card-bubble-title"><span className="clamp-2">{message.linkTitle || withoutLabel(text)}</span></strong>
        <small className="card-bubble-foot"><VideoCamera size={12} />{t('视频号{value}', { value: message.finderNickname ? ' · ' + message.finderNickname : '' })}</small>
      </Card>;
    case 'record':
      return <Record message={message} />;
    case 'contact':
      return <div className="message-bubble card-bubble contact-bubble">
        <span className="contact-line">{message.cardAvatarUrl ? <RemoteImage className="avatar" src={message.cardAvatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true">{Array.from(message.cardNickname ?? '?')[0]}</span>}<strong className="clamp-1">{message.cardNickname || withoutLabel(text)}</strong></span>
        <small className="card-bubble-foot"><UserSquare size={12} />{t('个人名片')}</small>
      </div>;
    case 'location':
      return <div className="message-bubble card-bubble location-bubble">
        <strong className="card-bubble-title"><MapPin size={16} />{message.locationPoiname || withoutLabel(text) || t('位置')}</strong>
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

/** 服务端给的缩略图，上面叠着 `badge`；无法解码时显示说明无法显示的占位，而不是破图。占位只在它代表的视频 `video`
 * 仍能播放时带 `badge`。 */
function MediaThumb({ src, label, video, badge }: { src: string; label: string; video?: string; badge?: ReactNode }) {
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading');
  const [plays, setPlays] = useState(false);
  useEffect(() => {
    if (state !== 'failed' || !video) return;
    let live = true;
    videoPlays(video).then(answer => { if (live) setPlays(answer.plays); });
    return () => { live = false; };
  }, [state, video]);
  if (state === 'failed') {
    return plays
      ? <span className="media-missing">{badge}<small>{t('缩略图无法显示')}</small></span>
      : <span className="media-missing"><Warning size={20} /><small>{t('{label}无法显示', { label })}</small></span>;
  }
  return <><img className={'message-thumb' + (state === 'loading' ? ' loading' : '')} src={src} alt={label} loading="lazy" decoding="async"
    onLoad={() => setState('ready')} onError={() => setState('failed')} />{badge}</>;
}
