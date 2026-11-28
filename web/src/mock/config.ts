// 示例数据的配置（#896）：档案和它们的设置，按 `config` 的方式读写，在页面打开期间保存在页面里。
// 默认档案已经用示例账号设置好；新加的档案从空的开始，按真实档案的步骤设置（查找数据目录、取得密钥、打开数据库），
// 每一步都会成功。路径都是虚构的，不属于任何人的电脑。
import type { AccountCleared, ConfigWritten, DbAccounts, DbCandidates, DbConnection, ImageKeys, ProfileSummary, RemovalPlan } from '../api/contract.gen';
import { ME } from './people';

type Settings = Record<string, string>;

/** 示例数据的微信数据目录，虚构的。 */
export const DATA_DIR = 'C:\\Users\\sample\\Documents\\xwechat_files';
const ACCOUNT_DIR = DATA_DIR + '\\' + ME + '_5f3a';
const CONFIG_PATH = 'C:\\Users\\sample\\AppData\\Roaming\\weflow\\config.json';
const SAMPLE_KEY = '5a17c0de'.repeat(8);

/** 是否是服务端的设置而不是档案的设置：这些设置决定 `weflow serve` 怎样启动。 */
const isServeKey = (key: string) => key.startsWith('serve_') || key === 'auto_download_whitelist';

const IMAGE_KEYS = ['image_xor_key', 'image_aes_key'];
const ACCOUNT_KEYS = ['db_path', 'wxid', 'decrypt_key', ...IMAGE_KEYS];

/** 用示例账号设置好的档案的设置。 */
const configured = (): Settings => ({
  db_path: DATA_DIR, wxid: ME, decrypt_key: SAMPLE_KEY, image_xor_key: '90', image_aes_key: '6c3f9d1e2b8a4f70',
  ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'sample-model', ai_model_api_key: 'sample', ai_insight_enabled: 'true',
  ai_insight_allow_context: 'true',
  // 保存为传给 `config set` 的文字；服务端保存的是数组
  ai_insight_filter_list: JSON.stringify(['wxid_sample_linmu', 'wxid_sample_chenyu', 'wxid_sample_xiaolu']),
});

const profiles = new Map<string, Settings>([['default', configured()]]);
const serve: Settings = {};
let current = 'default';

const settings = () => profiles.get(current) ?? {};

/** 示例数据的档案的 `config profiles`。 */
export function profileSummaries(): ProfileSummary[] {
  return [...profiles].map(([name, s]) => ({
    name, current: name === current, wxid: s.wxid ?? null, dbPath: s.db_path ?? null, decryptKey: Boolean(s.decrypt_key),
    imageKeys: IMAGE_KEYS.every(k => s[k]), aiKey: Boolean(s.ai_model_api_key), complete: Boolean(s.db_path && s.decrypt_key),
  }));
}

/** `config get <key>`：当前档案的设置，启动选项则为服务端的设置；没有设置时为 null。 */
export function getSetting(key: string): string | null {
  if (key === 'current_profile') return current;
  return (isServeKey(key) ? serve[key] : settings()[key]) ?? null;
}

const written = (): ConfigWritten => ({ configPath: CONFIG_PATH });

/** `config set <key> <value>`；`current_profile` 切换到这个档案，档案不存在时新建。 */
export function setSetting(key: string, value: string): ConfigWritten {
  if (key === 'current_profile') {
    if (!profiles.has(value)) profiles.set(value, {});
    current = value;
  } else if (isServeKey(key)) serve[key] = value;
  else settings()[key] = value;
  return written();
}

/** `config unset <key>`：去掉一个设置。 */
export function unsetSetting(key: string): ConfigWritten {
  if (isServeKey(key)) delete serve[key];
  else delete settings()[key];
  return written();
}

/** 见解引擎是否运行：与 `serve --insight` 一样按启动选项决定，这里默认打开。 */
export const engineRuns = () => serve.serve_insight !== 'false';

/** `db detect`：微信数据的常见位置，其中示例数据的目录存在。 */
export const detectDirs = (): DbCandidates => ({
  candidates: [{ path: DATA_DIR, exists: true }, { path: 'D:\\WeChat Files', exists: false }],
});

/** `db wxid <root>`：不论在哪个文件夹里都找到示例账号。 */
export const findAccounts = (root: string): DbAccounts => ({ accounts: [{ wxid: ME, path: root.replace(/[\\/]+$/, '') + '\\' + ME + '_5f3a' }] });

/** `db test`：档案设置了数据目录和密钥时能打开；否则给出 `config_error` 报告的问题。 */
export function testDatabase(): { data: DbConnection } | { problem: string } {
  const s = settings();
  if (!s.db_path || !s.wxid) return { problem: '当前档案没有设置 db_path 或 wxid' };
  if (!s.decrypt_key) return { problem: '当前档案没有设置 decrypt_key' };
  return { data: { accountDir: ACCOUNT_DIR } };
}

/** `key db --save` 取得密钥后要做的事：把密钥保存到档案里。 */
export function saveDatabaseKey() {
  settings().decrypt_key = SAMPLE_KEY;
  return { saved: ['decrypt_key'] };
}

/** `key image --save`：图片密钥，用示例数据的图片校验后保存到档案里。 */
export function saveImageKeys(): ImageKeys {
  const s = settings();
  s.image_xor_key = '90';
  s.image_aes_key = '6c3f9d1e2b8a4f70';
  return {
    verified: true, xor_verified: true, verification: 'jpeg_decode_and_xor_tail',
    sources: [{ path: 'C:\\Users\\sample\\AppData\\Roaming\\Tencent\\xwechat\\all_users\\kvcomm\\key_0.statistic', parsing: 'kvcomm' }],
    collection: { folders: 1, files: 4 }, scan: { images: 24, checked: 6 }, saved: IMAGE_KEYS,
  };
}

/** `cache clear-account` 为当前账号删除的文件。 */
const accountFiles = () => [
  { path: ACCOUNT_DIR + '\\weflow\\images', bytes: 48_234_496 },
  { path: ACCOUNT_DIR + '\\weflow\\voices', bytes: 6_291_456 },
  { path: ACCOUNT_DIR + '\\weflow\\analytics', bytes: 1_310_720 },
];

/** `cache clear-account --dry-run`：会删除什么，以及会去掉档案里的哪些键。 */
export function planClearAccount(): RemovalPlan {
  const paths = accountFiles();
  return { dryRun: true, paths, bytes: paths.reduce((n, p) => n + p.bytes, 0), profileKeys: ACCOUNT_KEYS.filter(k => settings()[k]) };
}

/** `cache clear-account --yes`：删除账号的文件，并从档案里去掉这个账号。 */
export function clearAccount(): AccountCleared {
  const s = settings();
  for (const key of ACCOUNT_KEYS) delete s[key];
  return { removedPaths: accountFiles().map(p => p.path), profileReset: true };
}
