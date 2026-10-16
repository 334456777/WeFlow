// The token of `weflow serve`, typed in (#214): for a page opened without it, or with one the server no longer takes.
// It is kept for this tab only (sessionStorage, see client.ts) and never shown again; the connection tries it at once.
import { useState } from 'react';
import type { FormEvent } from 'react';
import { Key } from '@phosphor-icons/react';
import { setToken } from './api/client';

export function TokenForm({ label = '令牌' }: { label?: string }) {
  const [value, setValue] = useState('');
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const typed = value.trim();
    if (!typed) return;
    setToken(typed);
    setValue('');
  };
  return <form className="token-form" onSubmit={submit}>
    <label className="token-input"><Key size={16} aria-hidden="true" />
      <input type="password" autoComplete="off" spellCheck={false} aria-label={label} placeholder="粘贴 weflow serve 的令牌" value={value} onChange={event => setValue(event.target.value)} />
    </label>
    <button type="submit" className="primary-button" disabled={!value.trim()}>连接</button>
  </form>;
}

/** What to do when the server refuses the token: type it, or open the page again from `weflow serve`. */
export function TokenPrompt() {
  return <div className="token-prompt" role="alert">
    <h2>需要令牌</h2>
    <p>weflow serve 拒绝了这个标签页的令牌，或者网页没有带令牌打开。可以在下面粘贴令牌；也可以重新打开 weflow serve 启动时给出的地址（带有 <code>?token=</code>），网页会自动取用。</p>
    <TokenForm />
    <p className="muted">令牌只保存在这个标签页里，关闭后清除，不会写入网页文件或浏览器的长期存储。</p>
  </div>;
}
