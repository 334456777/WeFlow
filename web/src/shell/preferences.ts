// 设置里为整个网页选择、各自保存在这个浏览器里的偏好：实时消息（#201）、导出时附带的媒体（#342，不选就没有）、
// 减少动画（#193）。浏览器里没有保存时用默认值。
import { useEffect, useState } from 'react';
import { MEDIA_KINDS, type MediaKind } from '../exports/ExportCards';

const LIVE_KEY = 'weflow.live';
const MEDIA_KEY = 'weflow.media';
const MOTION_KEY = 'weflow.reduced-motion';

/** 网页自己的设置，保存在这个浏览器里：实时消息、导出时附带的媒体、减少动画。 */
export interface Preferences {
  realtime: boolean;
  setRealtime(on: boolean): void;
  media: MediaKind[];
  setMedia(kinds: MediaKind[]): void;
  reducedMotion: boolean;
  setReducedMotion(on: boolean): void;
}

function read<T>(key: string, parse: (text: string | null) => T): T {
  try { return parse(localStorage.getItem(key)); } catch { return parse(null); }
}

function keep(key: string, text: string) {
  try { localStorage.setItem(key, text); } catch { /* 存不下时只是刷新后不记得 */ }
}

/** 保存在 `weflow.media` 下的媒体：只取这个网页认识的种类，按它们的顺序；读不懂时为空。 */
export function keptMedia(text: string | null): MediaKind[] {
  try {
    const kept: unknown = JSON.parse(text || '[]');
    return Array.isArray(kept) ? MEDIA_KINDS.map(k => k.value).filter(k => kept.includes(k)) : [];
  } catch {
    return [];
  }
}

/** 保存的设置，以及修改并保存它们的函数。 */
export function usePreferences(): Preferences {
  const [realtime, setLive] = useState(() => read(LIVE_KEY, text => text !== 'off'));
  const setRealtime = (on: boolean) => { setLive(on); keep(LIVE_KEY, on ? 'on' : 'off'); };
  const [media, setKinds] = useState(() => read(MEDIA_KEY, keptMedia));
  const setMedia = (kinds: MediaKind[]) => { setKinds(kinds); keep(MEDIA_KEY, JSON.stringify(kinds)); };
  const [reducedMotion, setMotion] = useState(() => read(MOTION_KEY, text => text === 'on'));
  const setReducedMotion = (on: boolean) => { setMotion(on); keep(MOTION_KEY, on ? 'on' : 'off'); };
  useEffect(() => {
    document.documentElement.classList.toggle('reduced-motion', reducedMotion);
  }, [reducedMotion]);
  return { realtime, setRealtime, media, setMedia, reducedMotion, setReducedMotion };
}
