// The cache of the settings page (#213), each part with its size, cleared after showing what goes.
import { useEffect, useState } from 'react';
import { CircleNotch, Trash } from '@phosphor-icons/react';
import { CACHE_LABELS, clearCache, listCache, planClearCache, type CachePart, type CachePartName } from '../api/settings';
import { formatSize } from '../jobs/TaskCenter';
import { GridSkeleton } from '../Skeleton';
import { NoteLine, reason, SettingsGroup, type Note } from './layout';
import { locale, t } from '../i18n';

/** `cache list`, and `cache clear --<part>` after `--dry-run` showed what goes. */
export function CacheSetting() {
  const [parts, setParts] = useState<CachePart[] | null>(null);
  const [round, setRound] = useState(0);
  const [plan, setPlan] = useState<{ part: CachePartName; paths: { path: string; bytes: number }[]; bytes: number } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<Note>(null);
  // the part a note is about: it shows under that part's row, by its button (#346); null under the whole list
  const [noted, setNoted] = useState<CachePartName | null>(null);
  const tell = (text: Note, part: CachePartName | null) => { setNote(text); setNoted(part); };
  useEffect(() => {
    const controller = new AbortController();
    listCache(controller.signal).then(list => setParts(Array.isArray(list) ? list : []), problem => {
      if (!controller.signal.aborted) { setParts([]); tell({ text: reason(problem, t('读不到缓存。')), error: true }, null); }
    });
    return () => controller.abort();
  }, [round]);
  const look = async (part: CachePartName) => {
    setBusy('plan:' + part); tell(null, null);
    try { setPlan({ part, ...(await planClearCache(part)) }); }
    catch (problem) { tell({ text: reason(problem, t('读不到将要清理的内容。')), error: true }, part); }
    finally { setBusy(null); }
  };
  const clear = async () => {
    if (!plan) return;
    setBusy('clear'); tell(null, null);
    try {
      await clearCache(plan.part);
      tell({ text: t('已清理{title}，释放 {size}。', { title: t(CACHE_LABELS[plan.part].title), size: formatSize(plan.bytes) }) }, plan.part);
      setPlan(null); setRound(n => n + 1);
    } catch (problem) {
      tell({ text: reason(problem, t('清理失败，请重试。')), error: true }, plan.part);
    } finally {
      setBusy(null);
    }
  };
  const total = parts?.reduce((sum, p) => sum + p.bytes, 0) ?? 0;

  return <SettingsGroup title={t('缓存的文件')} note={parts && t('共 {size}', { size: formatSize(total) })}>
    {!parts ? <GridSkeleton label={t('正在读取缓存')} cells={6} /> : <ul className="cache-list">
      {parts.map(p => <li key={p.part}>
        <span className="account-copy"><strong>{CACHE_LABELS[p.part] ? t(CACHE_LABELS[p.part].title) : p.part}</strong><span className="cache-text">{CACHE_LABELS[p.part] && t(CACHE_LABELS[p.part].text)}</span></span>
        <span className="cache-size num">{formatSize(p.bytes)}{p.files ? ' · ' + t('{n} 个文件', { n: p.files.toLocaleString(locale()) }) : ''}</span>
        <button type="button" className="secondary-button" aria-label={t('清理{title}', { title: CACHE_LABELS[p.part] ? t(CACHE_LABELS[p.part].title) : p.part })} disabled={!p.bytes || busy !== null} onClick={() => look(p.part)}>
          {busy === 'plan:' + p.part ? <CircleNotch size={12} className="spin" /> : <Trash size={12} />}{t('清理')}</button>
        {plan && plan.part === p.part && <ClearPlan plan={plan} busy={busy === 'clear'} onCancel={() => setPlan(null)} onClear={clear} />}
        {noted === p.part && <NoteLine note={note} />}
      </li>)}
    </ul>}
    {noted === null && <NoteLine note={note} />}
  </SettingsGroup>;
}

/** What `cache clear --dry-run` says would go, under the row of its part, with the way to go on. */
function ClearPlan({ plan, busy, onCancel, onClear }: {
  plan: { part: CachePartName; paths: { path: string; bytes: number }[]; bytes: number }; busy: boolean; onCancel(): void; onClear(): void;
}) {
  return <div className="clear-plan" role="group" aria-label={t('将要清理的内容')}>
    <p>{t('清理{title}：删除下面的文件，共 {bytes}。', { title: t(CACHE_LABELS[plan.part].title), bytes: formatSize(plan.bytes) })}</p>
    <ul>{plan.paths.map(p => <li key={p.path}><span className="num">{formatSize(p.bytes)}</span><span className="account-path" title={p.path}>{p.path}</span></li>)}</ul>
    <div className="clear-plan-actions">
      <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>{t('取消')}</button>
      <button type="button" className="primary-button danger" disabled={busy} onClick={onClear}>{busy ? <CircleNotch size={14} className="spin" /> : <Trash size={14} />}{t('确认清理')}</button>
    </div>
  </div>;
}
