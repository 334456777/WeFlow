// 数据库备份（#212）：`backup create` 作为作业运行，生成的备份保留下来供浏览器下载；`backup inspect` 和
// `backup restore` 读取服务端所在电脑上的备份（浏览器不能把备份传给服务端）。
import { get } from './client';
import { msg, t } from '../i18n';
import type * as Contract from './contract.gen';

/** 备份里的一个文件。 */
export type BackupEntry = Contract.BackupEntry;

/** `backup inspect` 从备份里读到的清单（契约的类型，#430）。 */
export type BackupManifest = Contract.BackupManifest;

/** `backup inspect`：服务端所在电脑上 `path` 处的备份的清单。 */
export const inspectBackup = (path: string, signal?: AbortSignal) => get<BackupManifest>('backup/inspect', { path }, signal);

/** `backup create` 在数据库之外加进备份的内容；数据库总会备份（语音消息就在数据库里，#643）。 */
export type BackupChoice = { images: boolean; videos: boolean; files: boolean; emojis: boolean };

/** `backup create` 的参数：在数据库之外加进备份的内容。 */
export function backupParams(parts: BackupChoice) {
  return {
    images: parts.images || undefined,
    videos: parts.videos || undefined,
    files: parts.files || undefined,
    emojis: parts.emojis || undefined,
  };
}

/** 备份缺少的文件和可能不是同一时刻的数据库（#817），即 `backup create` 的回答和备份清单里记下的这两项。 */
export type BackupGaps = { skipped?: readonly Contract.SkippedFile[]; unsettled?: readonly string[] };

/** 最多列出的路径数，其余的只算进总数，用「等」带过。 */
const SHOWN = 3;

/** 说明备份缺少什么的句子：没有备份进去的文件，以及复制时微信一直在写入的数据库。 */
export function backupGaps({ skipped = [], unsettled = [] }: BackupGaps): string[] {
  const list = (paths: readonly string[]) => paths.slice(0, SHOWN).join(t('、')) + (paths.length > SHOWN ? t('等') : '');
  const lines: string[] = [];
  if (skipped.length) lines.push(t('{n} 个文件读不了，没有备份进去：{paths}', { n: skipped.length, paths: list(skipped.map(s => s.path)) }));
  if (unsettled.length) lines.push(t('{n} 个数据库在复制时一直被微信写入，备份里的数据库和它的 WAL 可能不是同一时刻的：{paths}', { n: unsettled.length, paths: list(unsettled) }));
  return lines;
}

/** 按内容分组的备份文件：数据库、微信的媒体文件和 WeFlow 的表情。 */
export function backupParts(entries: readonly BackupEntry[]): { label: string; files: number; bytes: number }[] {
  const groups = new Map<string, { files: number; bytes: number }>();
  for (const e of entries) {
    const path = e.path.replace(/\\/g, '/').toLowerCase();
    // 与 `backup create` 收集的目录一致：db_storage、msg/attach、msg/video、msg/file 和 WeFlow 的表情；
    // 较早的 WeFlow 做的备份里还可能有 FileStorage/Image(2) 和 FileStorage/Audio。标签显示时经过 t()
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
