// 示例数据的数据库备份，格式与 `backup inspect` 读到的一致：数据库，以及一些图片、文件、视频和表情。
import type { BackupEntry, BackupManifest } from '../api/contract.gen';
import { ago } from './clock';
import { ME } from './people';

const entry = (path: string, size: number, i: number): BackupEntry => ({ path, size, sha256: (i * 2_654_435_761 >>> 0).toString(16).padStart(8, '0').repeat(8) });

/** 示例备份的 `backup inspect`。 */
export function backupManifest(): BackupManifest {
  const files: [string, number][] = [
    ['db_storage/contact/contact.db', 2_883_584], ['db_storage/session/session.db', 1_048_576],
    ['db_storage/message/message_0.db', 186_646_528], ['db_storage/message/message_0.db-wal', 4_194_304],
    ['db_storage/message/message_1.db', 92_274_688], ['db_storage/message/media_0.db', 33_554_432],
    ['db_storage/sns/sns.db', 12_582_912], ['db_storage/head_image/head_image.db', 8_388_608],
    ...Array.from({ length: 24 }, (_, i): [string, number] => ['msg/attach/9f2c/2026-11/Img/' + String(i).padStart(4, '0') + '.dat', 180_000 + i * 7_300]),
    ['msg/file/2026-11/WeFlow 6.0 发布说明.md', 12_480], ['msg/file/2026-11/flamegraph.svg', 1_843_200],
    ['msg/file/2026-11/WebUI 设计稿.pdf', 3_250_585], ['msg/video/2026-11/0001.mp4', 6_291_456],
    ...Array.from({ length: 6 }, (_, i): [string, number] => ['emojis/' + String(i).padStart(2, '0') + '.gif', 42_000 + i * 900]),
  ];
  return {
    version: '1', created_at: ago(2, 3), wxid: ME, weflow_version: '6.0.0',
    entries: files.map(([path, size], i) => entry(path, size, i + 1)),
    skipped: [{ path: 'msg/attach/9f2c/2026-11/Img/0025.dat', reason: 'the file is in use' }],
  };
}
