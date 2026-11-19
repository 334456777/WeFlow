// The sample's configuration (#896): its profiles and their settings, as `config` reads and writes them, kept in the
// page while it is open. The default profile is set up with the sample account; one added starts empty and is set up
// through the steps a real one is (finding the data directory, the keys, opening the database), which all succeed.
// The paths are made up, on a computer of no one.
import type { AccountCleared, ConfigWritten, DbAccounts, DbCandidates, DbConnection, ImageKeys, ProfileSummary, RemovalPlan } from '../api/contract.gen';
import { ME } from './people';

type Settings = Record<string, string>;

/** The sample's WeChat data directory, made up. */
export const DATA_DIR = 'C:\\Users\\sample\\Documents\\xwechat_files';
const ACCOUNT_DIR = DATA_DIR + '\\' + ME + '_5f3a';
const CONFIG_PATH = 'C:\\Users\\sample\\AppData\\Roaming\\weflow\\config.json';
const SAMPLE_KEY = '5a17c0de'.repeat(8);

/** The settings of the server, not of a profile: how `weflow serve` starts. */
const isServeKey = (key: string) => key.startsWith('serve_') || key === 'autoDownloadWhitelist';

const IMAGE_KEYS = ['image_xor_key', 'image_aes_key'];
const ACCOUNT_KEYS = ['db_path', 'wxid', 'decrypt_key', ...IMAGE_KEYS];

/** The settings of a profile set up with the sample account. */
const configured = (): Settings => ({
  db_path: DATA_DIR, wxid: ME, decrypt_key: SAMPLE_KEY, image_xor_key: '90', image_aes_key: '6c3f9d1e2b8a4f70',
  ai_model_api_base_url: 'https://api.example.com/v1', ai_model_api_model: 'sample-model', ai_model_api_key: 'sample', ai_insight_enabled: 'true',
  aiInsightAllowContext: 'true',
});

const profiles = new Map<string, Settings>([['default', configured()]]);
const serve: Settings = {};
let current = 'default';

const settings = () => profiles.get(current) ?? {};

/** `config profiles` of the sample's profiles. */
export function profileSummaries(): ProfileSummary[] {
  return [...profiles].map(([name, s]) => ({
    name, current: name === current, wxid: s.wxid ?? null, dbPath: s.db_path ?? null, decryptKey: Boolean(s.decrypt_key),
    imageKeys: IMAGE_KEYS.every(k => s[k]), aiKey: Boolean(s.ai_model_api_key), complete: Boolean(s.db_path && s.decrypt_key),
  }));
}

/** `config get <key>`: the current profile's, or the server's for a startup option; null when it is not set. */
export function getSetting(key: string): string | null {
  if (key === 'current_profile') return current;
  return (isServeKey(key) ? serve[key] : settings()[key]) ?? null;
}

const written = (): ConfigWritten => ({ configPath: CONFIG_PATH });

/** `config set <key> <value>`; `current_profile` switches to the profile, made when it is new. */
export function setSetting(key: string, value: string): ConfigWritten {
  if (key === 'current_profile') {
    if (!profiles.has(value)) profiles.set(value, {});
    current = value;
  } else if (isServeKey(key)) serve[key] = value;
  else settings()[key] = value;
  return written();
}

/** `config unset <key>`. */
export function unsetSetting(key: string): ConfigWritten {
  if (isServeKey(key)) delete serve[key];
  else delete settings()[key];
  return written();
}

/** Whether the insight engine runs: as `serve --insight` would, from the startup option, on by default here. */
export const engineRuns = () => serve.serve_insight !== 'false';

/** `db detect`: the usual places of WeChat's data, the sample's found. */
export const detectDirs = (): DbCandidates => ({
  candidates: [{ path: DATA_DIR, exists: true }, { path: 'D:\\WeChat Files', exists: false }],
});

/** `db wxid <root>`: the sample account, in whichever folder. */
export const findAccounts = (root: string): DbAccounts => ({ accounts: [{ wxid: ME, path: root.replace(/[\\/]+$/, '') + '\\' + ME + '_5f3a' }] });

/** `db test`: opens when the profile has a data directory and a key; the problem otherwise, as `config_error` says it. */
export function testDatabase(): { data: DbConnection } | { problem: string } {
  const s = settings();
  if (!s.db_path || !s.wxid) return { problem: '当前档案没有设置 db_path 或 wxid' };
  if (!s.decrypt_key) return { problem: '当前档案没有设置 decrypt_key' };
  return { data: { accountDir: ACCOUNT_DIR, connected: true } };
}

/** What `key db --save` does once it has the key: saves it in the profile. */
export function saveDatabaseKey() {
  settings().decrypt_key = SAMPLE_KEY;
  return { saved: ['decrypt_key'] };
}

/** `key image --save`: the image keys, checked on the sample's pictures and saved in the profile. */
export function saveImageKeys(): ImageKeys {
  const s = settings();
  s.image_xor_key = '90';
  s.image_aes_key = '6c3f9d1e2b8a4f70';
  return {
    verified: true, aes_verified: true, xor_verified: true, verification: 'jpeg_decode_and_xor_tail',
    sources: [{ path: 'C:\\Users\\sample\\AppData\\Roaming\\Tencent\\xwechat\\all_users\\kvcomm\\key_0.statistic', parsing: 'kvcomm' }],
    collection: { folders: 1, files: 4 }, scan: { images: 24, checked: 6 }, saved: IMAGE_KEYS,
  };
}

/** The files `cache clear-account` takes for the current account. */
const accountFiles = () => [
  { path: ACCOUNT_DIR + '\\weflow\\images', bytes: 48_234_496 },
  { path: ACCOUNT_DIR + '\\weflow\\voices', bytes: 6_291_456 },
  { path: ACCOUNT_DIR + '\\weflow\\analytics', bytes: 1_310_720 },
];

/** `cache clear-account --dry-run`: what it would remove, and the profile keys it would unset. */
export function planClearAccount(): RemovalPlan {
  const paths = accountFiles();
  return { dryRun: true, paths, bytes: paths.reduce((n, p) => n + p.bytes, 0), profileKeys: ACCOUNT_KEYS.filter(k => settings()[k]) };
}

/** `cache clear-account --yes`: removes the account's files and takes the account out of the profile. */
export function clearAccount(): AccountCleared {
  const s = settings();
  for (const key of ACCOUNT_KEYS) delete s[key];
  return { removedPaths: accountFiles().map(p => p.path), profileReset: true };
}
