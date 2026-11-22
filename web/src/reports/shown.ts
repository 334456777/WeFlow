// Which report the reports page shows (#1077): the console's save card saves only a report that is there, and lets go
// of what it said of the one before.
import { useEffect, useRef } from 'react';

/** Tells `onShown` the report shown by a key of its own, null while there is none, and null once it goes. */
export function useShownReport(key: string | null, onShown: (key: string | null) => void): void {
  const latest = useRef(onShown);
  latest.current = onShown;
  useEffect(() => {
    latest.current(key);
    return () => latest.current(null);
  }, [key]);
}
