// What the settings choose for the whole page, each kept in this browser: live messages (#201), the media exports
// copy (#342, none unless picked) and reduced motion (#193). A browser that keeps nothing starts from the defaults.
import { useEffect, useState } from 'react';
import { MEDIA_KINDS, type MediaKind } from '../exports/ExportCards';

const LIVE_KEY = 'weflow.live';
const MEDIA_KEY = 'weflow.media';
const MOTION_KEY = 'weflow.reduced-motion';

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
  try { localStorage.setItem(key, text); } catch { /* only lost on reload */ }
}

/** The media kept under `weflow.media`: the kinds this page knows, in their order; none when unreadable. */
export function keptMedia(text: string | null): MediaKind[] {
  try {
    const kept: unknown = JSON.parse(text || '[]');
    return Array.isArray(kept) ? MEDIA_KINDS.map(k => k.value).filter(k => kept.includes(k)) : [];
  } catch {
    return [];
  }
}

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
