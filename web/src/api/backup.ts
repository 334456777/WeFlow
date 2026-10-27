// Database backups (#212): `backup create` as a job that keeps the backup for the browser, `backup inspect` and
// `backup restore`, which read a backup on the server's computer (the browser cannot hand one over yet).
import { get } from './client';
import type * as Contract from './contract.gen';

export type BackupEntry = Contract.BackupEntry;

/** What `backup inspect` reads from a backup (the contract's, #430). */
export type BackupManifest = Contract.BackupManifest;

export const inspectBackup = (path: string, signal?: AbortSignal) => get<BackupManifest>('backup/inspect', { path }, signal);

/** The parameters of `backup create`: what it leaves out. */
export function backupParams(parts: { images: boolean; voice: boolean; emojis: boolean }) {
  return {
    'no-images': parts.images ? undefined : true,
    'no-voice': parts.voice ? undefined : true,
    'no-emojis': parts.emojis ? undefined : true,
  };
}

/** The entries of a backup by what they are: the databases, and each part of the cache. */
export function backupParts(entries: readonly BackupEntry[]): { label: string; files: number; bytes: number }[] {
  const groups = new Map<string, { files: number; bytes: number }>();
  for (const e of entries) {
    const path = e.path.replace(/\\/g, '/').toLowerCase();
    // as `backup create` collects them: db_storage, FileStorage/Image(2), FileStorage/Audio, and WeFlow's emojis
    const label = path.startsWith('db_storage/') ? '数据库'
      : /^filestorage\/image2?\//.test(path) ? '图片'
        : path.startsWith('filestorage/audio/') ? '语音'
          : path.startsWith('emojis/') ? '表情'
            : '其他';
    const g = groups.get(label) ?? { files: 0, bytes: 0 };
    g.files += 1; g.bytes += e.size;
    groups.set(label, g);
  }
  const order = ['数据库', '图片', '语音', '表情', '其他'];
  return [...groups.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([label, g]) => ({ label, ...g }));
}
