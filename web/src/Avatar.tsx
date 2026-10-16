import { useState } from 'react';

/**
 * A picture, or the first character of the name on a raised disc when there is none or it does not load (WeChat's
 * avatar links expire). The pictures come from WeChat's servers, so no referrer is sent with them.
 */
export function Avatar({ name, url }: { name: string; url?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  return url && failed !== url
    ? <img className="avatar" src={url} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(url)} />
    : <span className="avatar avatar-initial" aria-hidden="true">{Array.from(name.trim())[0] ?? '?'}</span>;
}
