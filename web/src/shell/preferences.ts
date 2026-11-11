// What the settings choose for the whole page: live messages (#201, remembered), the media exports copy (#342, none
// unless asked for) and reduced motion (#193).
import { useEffect, useState } from 'react';
import type { MediaKind } from '../exports/ExportCards';

const LIVE_KEY = 'weflow.live';

export interface Preferences {
  realtime: boolean;
  setRealtime(on: boolean): void;
  media: MediaKind[];
  setMedia(kinds: MediaKind[]): void;
  reducedMotion: boolean;
  setReducedMotion(on: boolean): void;
}

export function usePreferences(): Preferences {
  const [realtime, setLive] = useState(() => { try { return localStorage.getItem(LIVE_KEY) !== 'off'; } catch { return true; } });
  const setRealtime = (on: boolean) => {
    setLive(on);
    try { localStorage.setItem(LIVE_KEY, on ? 'on' : 'off'); } catch { /* only lost on reload */ }
  };
  const [media, setMedia] = useState<MediaKind[]>([]);
  const [reducedMotion, setReducedMotion] = useState(false);
  useEffect(() => {
    document.documentElement.classList.toggle('reduced-motion', reducedMotion);
  }, [reducedMotion]);
  return { realtime, setRealtime, media, setMedia, reducedMotion, setReducedMotion };
}
