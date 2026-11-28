// 手动输入 `weflow serve` 的令牌（#214）：网页打开时没有带令牌，或者带的令牌服务端不再接受时使用。
// 令牌只保存在这个标签页里（sessionStorage，见 client.ts），之后不再显示；连接会立即用它重试。
import { useState } from 'react';
import type { FormEvent } from 'react';
import { Key } from '@phosphor-icons/react';
import { setToken } from './api/client';
import { t } from './i18n';

/** 输入令牌的输入框和设置令牌的按钮。 */
export function TokenForm({ label = t('令牌') }: { label?: string }) {
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
      <input type="password" autoComplete="off" spellCheck={false} aria-label={label} placeholder={t('粘贴 weflow serve 的令牌')} value={value} onChange={event => setValue(event.target.value)} />
    </label>
    <button type="submit" className="primary-button" disabled={!value.trim()}>{t('连接')}</button>
  </form>;
}

/** 服务端拒绝令牌时的提示：输入令牌，或者用 `weflow serve` 给出的地址重新打开网页。 */
export function TokenPrompt() {
  return <div className="token-prompt" role="alert">
    <h2>{t('需要令牌')}</h2>
    <p>{t('weflow serve 拒绝了这个标签页的令牌，或者网页没有带令牌打开。可以在下面粘贴令牌；也可以重新打开 weflow serve 启动时给出的地址（带有 ?token=），网页会自动取用。')}</p>
    <TokenForm />
    <p className="muted">{t('令牌只保存在这个标签页里，关闭后清除，不会写入网页文件或浏览器的长期存储。')}</p>
  </div>;
}
