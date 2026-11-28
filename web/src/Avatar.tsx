import { useState } from 'react';

/**
 * 头像图片；没有图片或者图片加载失败时（微信的头像链接会过期），在圆片上显示名字的第一个字。
 * 图片来自微信的服务器，所以请求时不发送 referrer。
 */
export function Avatar({ name, url }: { name: string; url?: string }) {
  const [failed, setFailed] = useState<string | null>(null);
  const initial = Array.from(name.trim())[0] ?? '?';
  // 名字的第一个字随图片一起放在 `data-initial` 里，供不带图片的页面副本（保存的报告）使用
  return url && failed !== url
    ? <img className="avatar" src={url} alt="" data-initial={initial} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(url)} />
    : <span className="avatar avatar-initial" aria-hidden="true">{initial}</span>;
}
