// The WebUI versions in the settings (#406): those kept on the server's machine, the published ones once asked for,
// picking one (the page reloads into it when the server shows it at once) and removing the old ones.
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { ArrowsClockwise, CircleNotch, Trash } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { WEB, type Health } from '../api/backend';
import { listKept, listPublished, pickVersion, pruneVersions, shownVersion, type KeptVersion, type PublishedVersion } from '../api/webVersions';
import { formatSize } from '../jobs/TaskCenter';
import { SettingRow, SettingsGroup } from './layout';
import { t } from '../i18n';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;
type Note = { text: string; error?: boolean } | null;

/** A version as a row: what the machine keeps of it and what the registry says, either or both. */
type Row = { version: string; kept?: KeptVersion; published?: PublishedVersion };

/** Rows shown before the rest are asked for. */
const FIRST = 6;

/** The published versions in the registry's order (newest first), then those kept that it no longer lists. */
function rows(kept: KeptVersion[], published: PublishedVersion[] | null): Row[] {
  const byVersion = new Map(kept.map(k => [k.version, k]));
  if (!published) return kept.map(k => ({ version: k.version, kept: k }));
  const listed = new Set(published.map(p => p.version));
  return [
    ...published.map(p => ({ version: p.version, published: p, kept: byVersion.get(p.version) })),
    ...kept.filter(k => !listed.has(k.version)).map(k => ({ version: k.version, kept: k })),
  ];
}

/** The newest published version that suits this weflow, a release before a pre-release, as `web update` picks. */
function newest(published: PublishedVersion[] | null): string | null {
  const suitable = (published ?? []).filter(p => p.suits);
  return (suitable.find(p => !p.version.includes('-')) ?? suitable[0])?.version ?? null;
}

/** Why a version does not suit this weflow, in the page's language (the server's `why` is in English); null when it suits. */
function why(row: Row, health: Health): string | null {
  const suits = row.published ? row.published.suits : row.kept?.suits ?? true;
  if (suits) return null;
  const apiVersion = row.published ? row.published.apiVersion : row.kept?.apiVersion;
  const minBackendVersion = row.published ? row.published.minBackendVersion : row.kept?.minBackendVersion;
  if (apiVersion && apiVersion !== health.apiVersion) return t('需要 API {need}，这个 weflow 提供 API {have}', { need: apiVersion, have: health.apiVersion });
  if (minBackendVersion) return t('需要 weflow {need} 或更新的版本，这个是 {have}', { need: minBackendVersion, have: health.version });
  return t('不适用于这个 weflow');
}

export function WebVersions({ health, onReload = () => window.location.reload() }: { health: Health; onReload?(): void }) {
  const [kept, setKept] = useState<KeptVersion[] | null>(null);
  const [published, setPublished] = useState<PublishedVersion[] | null>(null);
  const [round, setRound] = useState(0);
  const [all, setAll] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<Note>(null);
  useEffect(() => {
    const controller = new AbortController();
    listKept(controller.signal).then(list => setKept(Array.isArray(list) ? list : []), problem => {
      if (!controller.signal.aborted) { setKept([]); setNote({ text: reason(problem, t('读不到本机的网页版本。')), error: true }); }
    });
    return () => controller.abort();
  }, [round]);

  const check = async () => {
    setBusy('check'); setNote(null);
    try {
      const list = await listPublished();
      setPublished(Array.isArray(list) ? list : []);
      const latest = newest(list);
      setNote({ text: latest === null ? t('没有适用于这个 weflow 的已发布版本。') : latest === WEB.version ? t('已经是最新的适用版本。') : t('有新的适用版本 {version}。', { version: latest }) });
    } catch (problem) {
      setNote({ text: reason(problem, t('读不到已发布的版本。')), error: true });
    } finally {
      setBusy(null);
    }
  };
  const pick = async (version: string) => {
    setBusy('pick:' + version); setNote(null);
    try {
      const chosen = await pickVersion(version);
      if (await shownVersion() === chosen.version) {
        setNote({ text: t('已换成 {version}，正在重新打开网页。', { version: chosen.version }) });
        onReload();
        return;
      }
      setNote({ text: t('已选定 {version}。这次 weflow serve 指定了要显示的网页，下次启动时换上。', { version: chosen.version }) });
      setRound(n => n + 1);
    } catch (problem) {
      setNote({ text: reason(problem, t('换版本失败，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };
  const prune = async () => {
    setBusy('prune'); setNote(null);
    try {
      const done = await pruneVersions();
      const removed = done.removed.length
        ? t('删除了 {n} 个版本：{versions}。', { n: done.removed.length, versions: done.removed.join(t('、')) })
        : t('没有可以删除的版本。');
      const shown = done.shown.length ? t('正在显示的 {versions} 等 weflow serve 停止后再删除。', { versions: done.shown.join(t('、')) }) : '';
      setNote({ text: removed + shown });
      setRound(n => n + 1);
    } catch (problem) {
      setNote({ text: reason(problem, t('清理失败，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };

  const list = kept ? rows(kept, published) : [];
  const latest = newest(published);
  const shown = all ? list : list.slice(0, FIRST);
  return <>
    <VersionInUse>
      <button type="button" className="secondary-button" disabled={busy !== null || !kept} onClick={check}>
        {busy === 'check' ? <CircleNotch size={12} className="spin" /> : <ArrowsClockwise size={12} />}{t('检查更新')}</button>
    </VersionInUse>
    <SettingsGroup title={t('可以换上的版本')} note={kept && !list.length ? t('本机没有保存网页版本；检查更新后列出已发布的版本。') : t('本机保存的版本；检查更新后加上已发布的版本。')} actions={<button type="button" className="secondary-button" disabled={busy !== null || !kept} onClick={prune}>
      {busy === 'prune' ? <CircleNotch size={12} className="spin" /> : <Trash size={12} />}{t('清理旧版本')}</button>}>
      {kept && list.length > 0 && <ul className="cache-list" aria-label={t('网页版本')}>
        {shown.map(row => {
          const current = row.version === WEB.version;
          const unsuitable = why(row, health);
          const tags = [
            current && t('正在使用'),
            !current && row.kept?.picked && t('已选定'),
            row.version === latest && t('最新'),
            row.kept ? t('已下载') : row.published && formatSize(row.published.size),
          ].filter(Boolean).join(' · ');
          return <li key={row.version}>
            <span className="account-copy"><strong className="num">{row.version}</strong>
              <span className="cache-text">{unsuitable ?? tags}</span></span>
            {!current && !unsuitable && <button type="button" className="secondary-button" disabled={busy !== null} aria-label={t('换成 {version}', { version: row.version })} onClick={() => pick(row.version)}>
              {busy === 'pick:' + row.version && <CircleNotch size={12} className="spin" />}{t('换上')}</button>}
          </li>;
        })}
      </ul>}
      {list.length > FIRST && <button type="button" className="secondary-button web-versions-more" onClick={() => setAll(v => !v)}>{all ? t('收起') : t('显示全部 {n} 个版本', { n: list.length })}</button>}
      {note && <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
    </SettingsGroup>
  </>;
}

/** The version this page is and what it needs; without the server, all there is to show. */
export function VersionInUse({ children }: { children?: ReactNode }) {
  return <SettingsGroup title={t('正在使用')}>
    <SettingRow name={<span className="num">{WEB.version}</span>} text={t('需要 API {apiVersion}，weflow {minBackendVersion} 或更新的版本。', { apiVersion: WEB.apiVersion, minBackendVersion: WEB.minBackendVersion })}>{children}</SettingRow>
  </SettingsGroup>;
}
