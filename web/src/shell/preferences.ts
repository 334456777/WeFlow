// What the settings choose for the whole page: live messages (#201, remembered), media in exports (#342, off unless
// asked for) and reduced motion (#193).
import { useEffect, useState } from 'react';

const LIVE_KEY = 'weflow.live';

export interface Preferences {
  realtime: boolean;
  setRealtime(on: boolean): void;
  includeMedia: boolean;
  setIncludeMedia(on: boolean): void;
  reducedMotion: boolean;
  setReducedMotion(on: boolean): void;
}

export function usePreferences(): Preferences {
  const [realtime, setLive] = useState(() => { try { return localStorage.getItem(LIVE_KEY) !== 'off'; } catch { return true; } });
  const setRealtime = (on: boolean) => {
    setLive(on);
    try { localStorage.setItem(LIVE_KEY, on ? 'on' : 'off'); } catch { /* only lost on reload */ }
  };
  const [includeMedia, setIncludeMedia] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  useEffect(() => {
    document.documentElement.classList.toggle('reduced-motion', reducedMotion);
  }, [reducedMotion]);
  return { realtime, setRealtime, includeMedia, setIncludeMedia, reducedMotion, setReducedMotion };
}
