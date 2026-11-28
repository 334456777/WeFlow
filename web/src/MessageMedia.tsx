// 聊天窗口之外也用到的消息的图片、视频和语音：预览对话框打开的内容（聊天窗口、资源浏览和朋友圈都会打开它）、
// 视频上的播放标记、缩略图显示不了的原因，以及播放语音的按钮（聊天窗口和资源浏览）。
import { useEffect, useRef, useState } from 'react';
import { Pause, Play, Warning } from '@phosphor-icons/react';
import { mediaUrl, type ParsedMessage } from './api/messages';
import { formatDuration } from './format';
import { t } from './i18n';

/** 预览对话框显示的内容：一张图片或一个视频，以及怎样切换到同一组里的其他图片和视频。 */
export type Preview =
  | { type: 'image'; src: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb; hd?: PreviewHd }
  | { type: 'video'; src: string; poster?: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb };

/**
 * 预览按打开时的样子（`.dat`）显示的图片的原图：微信点「查看原图」时下载的 `_h.dat`，下载到本机后改为显示它（#194）。
 * 请求过之后 `shown` 为 true，到达之前 `loading` 为 true，`opened` 是被它替换的图片，用来换回去（#338）。
 */
export interface PreviewHd { src: string; sessionId: string; localId: number; createTime: number; shown?: boolean; loading?: boolean; opened?: string }

/** 预览打开图片消息时的内容：微信打开图片时显示的那一张，以及它的原图。 */
export function imagePreview(sessionId: string, message: ParsedMessage, title: string): Extract<Preview, { type: 'image' }> {
  return {
    type: 'image', src: mediaUrl('image-data', sessionId, message, { variant: 'normal' }), title,
    hd: { src: mediaUrl('image-data', sessionId, message, { variant: 'hd' }), sessionId, localId: message.localId, createTime: message.createTime },
  };
}

/**
 * 打开预览时的缩略图（#194）：预览立即按它的形状排版，在图片到达之前显示它，
 * 这样从缩略图飞出的图片有落脚的位置，加载出来后也不会再移动。
 */
export interface PreviewThumb { src: string; aspect: number }

/** 同一组里已经读到的图片和视频（如一个会话里的），当前显示的是第 `index` 个（#194）。 */
export interface Gallery { items: Preview[]; index: number }

/** 请求媒体地址的第一个字节，从回答知道文件在不在。 */
const firstByte = (src: string) => fetch(src, { headers: { Range: 'bytes=0-0' } });

/** 失败的回答里的原因：服务端以 JSON 回答错误。 */
async function failureIn(response: Response): Promise<string> {
  try {
    const doc = await response.json();
    return doc?.error?.message || t('无法读取（HTTP {status}）', { status: response.status });
  } catch {
    return t('无法读取');
  }
}

/** 媒体地址读取失败的原因：服务端以 JSON 回答错误（如视频还没下载、图片的格式不支持）。 */
export async function reasonOf(src: string): Promise<string> {
  try {
    return await failureIn(await firstByte(src));
  } catch {
    return t('无法读取');
  }
}

/** `src` 的视频能不能播放（请求它的第一个字节）；不能时给出原因。 */
export async function videoPlays(src: string): Promise<{ plays: boolean; reason: string }> {
  try {
    const answer = await firstByte(src);
    if (!answer.ok) return { plays: false, reason: await failureIn(answer) };
    await answer.body?.cancel();
    return { plays: true, reason: '' };
  } catch {
    return { plays: false, reason: t('无法读取') };
  }
}

/**
 * `src` 的缩略图失败的原因；给了 `video`（缩略图代表的视频的地址）时，还给出这个视频是否仍能播放。
 * 不在本机的视频，它自己和缩略图的失败原因相同，所以先问视频。
 */
export async function thumbFailure(src: string, video?: string): Promise<{ reason: string; plays: boolean }> {
  if (video) {
    const answer = await videoPlays(video);
    if (!answer.plays) return answer;
  }
  return { plays: Boolean(video), reason: await reasonOf(src) };
}

/** 画在视频缩略图上的播放标记；缩略图失败时代替警告图标。 */
export function PlayBadge({ size }: { size: number }) {
  return <span className="play-badge" aria-hidden="true"><Play size={size} weight="fill" /></span>;
}

/** 整个页面同时只播放一段语音。 */
let playing: { audio: HTMLAudioElement; stop(): void } | null = null;

/** 语音消息的气泡，按下时播放在服务端解码的语音。 */
export function Voice({ sessionId, message, className = 'message-bubble voice-bubble' }: { sessionId: string; message: ParsedMessage; className?: string }) {
  const [state, setState] = useState<'idle' | 'loading' | 'playing' | 'error'>('idle');
  const audio = useRef<HTMLAudioElement | null>(null);
  useEffect(() => () => { audio.current?.pause(); }, []);
  const seconds = message.voiceDurationSeconds ?? 0;
  const toggle = () => {
    if (state === 'playing' || state === 'loading') { audio.current?.pause(); setState('idle'); return; }
    playing?.stop();
    const sound = audio.current ?? new Audio(mediaUrl('voice-data', sessionId, message, {
      'server-id': message.serverId, sender: message.senderUsername ?? undefined,
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
  // 语音越长，气泡越长（最长 220px）。
  const width = Math.min(220, 72 + seconds * 4);
  return <button type="button" className={className + ' ' + state} style={className.includes('voice-bubble') ? { width } : undefined} onClick={toggle}
    aria-label={state === 'playing' ? t('暂停语音，{duration}', { duration: formatDuration(seconds) }) : t('播放语音，{duration}', { duration: formatDuration(seconds) })}>
    {state === 'error' ? <Warning size={16} /> : state === 'playing' ? <Pause size={16} weight="fill" /> : <Play size={16} weight="fill" />}
    <span className="voice-wave" aria-hidden="true" />
    <span className="num">{state === 'error' ? t('无法播放') : formatDuration(seconds)}</span>
  </button>;
}
