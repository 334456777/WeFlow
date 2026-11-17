// Database backups (#212): `backup create` as a job that keeps the backup for the browser, `backup inspect` and
// `backup restore`, which read a backup on the server's computer (the browser cannot hand one over yet).
import { get } from './client';
import { msg, t } from '../i18n';
import type * as Contract from './contract.gen';

export type BackupEntry = Contract.BackupEntry;

/** What `backup inspect` reads from a backup (the contract's, #430). */
export type BackupManifest = Contract.BackupManifest;

export const inspectBackup = (path: string, signal?: AbortSignal) => get<BackupManifest>('backup/inspect', { path }, signal);

/** What `backup create` adds to the databases, which it always holds (with the voice messages in them, #643). */
export type BackupChoice = { images: boolean; videos: boolean; files: boolean; emojis: boolean };

/** The parameters of `backup create`: what it adds to the databases. */
export function backupParams(parts: BackupChoice) {
  return {
    images: parts.images || undefined,
    videos: parts.videos || undefined,
    files: parts.files || undefined,
    emojis: parts.emojis || undefined,
  };
}

/** What a backup lacks or may not hold of one moment (#817), as `backup create` answers and its manifest keeps it. */
export type BackupGaps = { skipped?: readonly Contract.SkippedFile[]; unsettled?: readonly string[] };

/** The paths at most `shown` are listed of, the rest counted. */
const SHOWN = 3;

/** Sentences for what a backup lacks: the files left out, and the databases WeChat kept writing to while copied. */
export function backupGaps({ skipped = [], unsettled = [] }: BackupGaps): string[] {
  const list = (paths: readonly string[]) => paths.slice(0, SHOWN).join(t('、')) + (paths.length > SHOWN ? t('等') : '');
  const lines: string[] = [];
  if (skipped.length) lines.push(t('{n} 个文件读不了，没有备份进去：{paths}', { n: skipped.length, paths: list(skipped.map(s => s.path)) }));
  if (unsettled.length) lines.push(t('{n} 个数据库在复制时一直被微信写入，备份里的数据库和它的 WAL 可能不是同一时刻的：{paths}', { n: unsettled.length, paths: list(unsettled) }));
  return lines;
}

/** The entries of a backup by what they are: the databases, WeChat's media and WeFlow's stickers. */
export function backupParts(entries: readonly BackupEntry[]): { label: string; files: number; bytes: number }[] {
  const groups = new Map<string, { files: number; bytes: number }>();
  for (const e of entries) {
    const path = e.path.replace(/\\/g, '/').toLowerCase();
    // as `backup create` collects them: db_storage, msg/attach, msg/video, msg/file and WeFlow's stickers; a backup of
    // an older WeFlow can hold FileStorage/Image(2) and FileStorage/Audio. Shown through t()
    const label = path.startsWith('db_storage/') ? msg('数据库')
      : path.startsWith('msg/attach/') || /^filestorage\/image2?\//.test(path) ? msg('图片')
        : path.startsWith('msg/video/') ? msg('视频')
          : path.startsWith('msg/file/') ? msg('文件')
            : path.startsWith('filestorage/audio/') ? msg('语音')
              : path.startsWith('emojis/') ? msg('表情')
                : msg('其他');
    const g = groups.get(label) ?? { files: 0, bytes: 0 };
    g.files += 1; g.bytes += e.size;
    groups.set(label, g);
  }
  const order = [msg('数据库'), msg('图片'), msg('视频'), msg('文件'), msg('语音'), msg('表情'), msg('其他')];
  return [...groups.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([label, g]) => ({ label, ...g }));
}
