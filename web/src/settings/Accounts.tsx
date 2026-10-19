// The accounts of the settings (#215): each profile with its account and how far it is set up, switching to another,
// adding one (then the setup), and clearing what WeFlow keeps of the current account, shown before it asks.
import { useState } from 'react';
import type { FormEvent } from 'react';
import { CheckCircle, CircleNotch, Plus, Trash, Warning } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { clearAccount, planClearAccount, profileNameProblem, switchProfile, type ClearPlan, type Profile } from '../api/setup';
import { formatSize } from '../jobs/TaskCenter';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

/** How far a profile is set up, in a few words. */
export function profileState(p: Profile): string {
  if (!p.complete) return p.dbPath ? '缺少数据库密钥' : '还没有配置';
  return p.imageKeys ? '已配置' : '已配置，没有图片密钥';
}

export function Accounts({ profiles, onChanged, onSetup }: {
  profiles: Profile[];
  /** A profile was switched, added or cleared: read them again. */
  onChanged(): void;
  /** Open the setup for the current profile. */
  onSetup(): void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState('');
  const [plan, setPlan] = useState<ClearPlan | null>(null);
  const current = profiles.find(p => p.current);

  const run = async (what: string, action: () => Promise<unknown>, done: string) => {
    setBusy(what); setNote(null);
    try {
      await action();
      setNote({ text: done });
      onChanged();
      return true;
    } catch (problem) {
      setNote({ text: reason(problem, '没有完成，请重试。'), error: true });
      return false;
    } finally {
      setBusy(null);
    }
  };
  const switchTo = (p: Profile) => { setPlan(null); void run('switch:' + p.name, () => switchProfile(p.name), '已切换到档案 ' + p.name + '，各页会重新读取。'); };
  const problem = adding ? profileNameProblem(name, profiles) : null;
  const add = async (event: FormEvent) => {
    event.preventDefault();
    if (problem) return;
    const typed = name.trim();
    if (await run('add', () => switchProfile(typed), '已添加档案 ' + typed + '。')) {
      setAdding(false); setName(''); setPlan(null);
      onSetup();
    }
  };
  const lookAtClearing = async () => {
    setBusy('plan'); setNote(null);
    try { setPlan(await planClearAccount()); }
    catch (problem) { setNote({ text: reason(problem, '读不到将要清除的内容。'), error: true }); }
    finally { setBusy(null); }
  };
  const clear = () => { void run('clear', clearAccount, '已清除这个账号的数据，档案里的账号和密钥也已移除。').then(ok => { if (ok) setPlan(null); }); };

  return <section className="accounts" aria-label="账号">
    <ul className="account-list">
      {profiles.map(p => <li key={p.name} className={'account' + (p.current ? ' current' : '')}>
        <span className="account-copy">
          <strong>{p.name}{p.current && <span className="badge">当前</span>}</strong>
          <span className="account-id">{p.wxid || '未选择账号'}</span>
          {p.dbPath && <span className="account-path" title={p.dbPath}>{p.dbPath}</span>}
        </span>
        <span className={'account-state' + (p.complete ? ' ok' : '')}>{p.complete ? <CheckCircle size={14} weight="fill" /> : <Warning size={14} />}{profileState(p)}</span>
        {p.current
          ? <button type="button" className="secondary-button" onClick={onSetup}>配置向导</button>
          : <button type="button" className="secondary-button" disabled={busy !== null} onClick={() => switchTo(p)}>{busy === 'switch:' + p.name ? <CircleNotch size={12} className="spin" /> : null}切换</button>}
      </li>)}
    </ul>

    <div className="account-actions">
      {adding
        ? <form className="token-form" onSubmit={add}>
          <label className="token-input"><Plus size={16} aria-hidden="true" />
            <input type="text" spellCheck={false} autoFocus aria-label="新档案的名称" placeholder="新档案的名称，如 work" value={name} onChange={event => setName(event.target.value)} />
          </label>
          <button type="submit" className="secondary-button" disabled={Boolean(problem) || busy !== null}>添加并配置</button>
          <button type="button" className="text-button" onClick={() => { setAdding(false); setName(''); }}>取消</button>
        </form>
        : <button type="button" className="secondary-button" onClick={() => { setAdding(true); setPlan(null); }}><Plus size={14} />添加账号</button>}
      {!adding && current?.wxid && !plan && <button type="button" className="secondary-button" disabled={busy !== null} onClick={lookAtClearing}>
        {busy === 'plan' ? <CircleNotch size={12} className="spin" /> : <Trash size={14} />}清除账号数据</button>}
    </div>
    {adding && name.trim() && problem && <p className="setting-warning">{problem}</p>}

    {plan && <div className="clear-plan" role="group" aria-label="将要清除的内容">
      <p>清除档案 <strong>{current?.name}</strong> 的账号 <strong>{current?.wxid}</strong>：删除 WeFlow 为它保存的以下文件（共 {formatSize(plan.bytes)}），并从档案里移除 {plan.profileKeys.join('、')}。微信自己的文件不会被动。</p>
      {plan.paths.length
        ? <ul>{plan.paths.map(p => <li key={p.path}><span className="num">{formatSize(p.bytes)}</span><span className="account-path" title={p.path}>{p.path}</span></li>)}</ul>
        : <p className="muted">没有需要删除的文件，只会移除档案里的账号和密钥。</p>}
      <div className="clear-plan-actions">
        <button type="button" className="secondary-button" disabled={busy === 'clear'} onClick={() => setPlan(null)}>取消</button>
        <button type="button" className="primary-button danger" disabled={busy === 'clear'} onClick={clear}>{busy === 'clear' ? <CircleNotch size={14} className="spin" /> : <Trash size={14} />}确认清除</button>
      </div>
    </div>}
    {note && <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </section>;
}

/** While the profiles are read (#282): rows in the shape of the accounts, so the settings do not jump. */
export function AccountsSkeleton() {
  return <div className="accounts" aria-busy="true" aria-label="正在读取账号配置">
    <ul className="account-list">
      {[64, 48].map(w => <li key={w} className="account">
        <span className="account-copy"><span className="skeleton line" style={{ width: '20%' }} /><span className="skeleton line" style={{ width: '30%' }} /><span className="skeleton line" style={{ width: w + '%' }} /></span>
        <span className="skeleton line account-skeleton-state" />
      </li>)}
    </ul>
  </div>;
}
