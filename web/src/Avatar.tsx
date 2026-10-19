import { useState } from 'react';

/**
 * A picture, or the first character of the name on a raised disc when there is none or it does not load (WeChat's
 * avatar links expire). The pictures come from WeChat's servers, so no referrer is sent with them.
 */
export function Avatar({ name, url }: { name: string; url?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  const initial = Array.from(name.trim())[0] ?? '?';
  // the initial rides along on the picture, for copies of the page that leave pictures out (a saved report)
  return url && failed !== url
    ? <img className="avatar" src={url} alt="" data-initial={initial} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(url)} />
    : <span className="avatar avatar-initial" aria-hidden="true">{initial}</span>;
}
