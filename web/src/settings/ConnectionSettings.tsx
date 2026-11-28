// 设置里的连接（#214）：weflow serve 的状态和重新检测的按钮、它的版本和是否适用于这个网页、运行方式，以及这个
// 标签页保存的 token（不显示它的值）。
import { useEffect, useState } from 'react';
import { ArrowsClockwise } from '@phosphor-icons/react';
import type { Backend, Health } from '../api/backend';
import { onAuth, setToken, token } from '../api/client';
import { backendDetail, backendLabel } from '../shell/Connection';
import { TokenForm } from '../TokenForm';
import { SettingRow, SettingsGroup } from './layout';
import { t } from '../i18n';

/** 服务端的运行方式（#214）：同时运行的上限和见解引擎；服务端都没有给出时为 null。 */
function serverDetail(health: Health): string | null {
  const { limits, insight } = health;
  if (!limits && insight === undefined) return null;
  const parts: string[] = [];
  if (limits?.maxTasks !== undefined) parts.push(t('同时运行任务 {n} 个', { n: limits.maxTasks }));
  if (limits?.maxReads !== undefined) parts.push(t('读取 {n} 个', { n: limits.maxReads }));
  if (limits?.maxMedia !== undefined) parts.push(t('媒体解码 {n} 个', { n: limits.maxMedia }));
  if (insight !== undefined) parts.push(insight ? t('灵感引擎已开启') : t('灵感引擎未开启'));
  return parts.join(' · ');
}

/** 每种状态的圆点，与侧栏显示的一样。 */
const DOTS: Record<Backend['state'], string> = { checking: 'exporting', ready: 'done', offline: '', incompatible: 'error', unauthorized: 'error' };

/** 连接状态、服务端的版本和选项，以及 token。 */
export function ConnectionSettings({ backend, recheck }: { backend: Backend; recheck(): void }) {
  const [tokenSaved, setTokenSaved] = useState(() => Boolean(token()));
  useEffect(() => onAuth(event => { if (event === 'token') setTokenSaved(Boolean(token())); }), []);
  const health = 'health' in backend ? backend.health : null;
  const running = health && serverDetail(health);
  return <>
    <SettingsGroup title="weflow serve" actions={<button className="secondary-button" type="button" onClick={recheck}><ArrowsClockwise size={12} />{t('重新检测')}</button>}>
      {/* 先是状态；服务端给出版本后显示版本，否则显示状态的含义 */}
      <SettingRow name={<span className="settings-state"><span className={'status-dot ' + DOTS[backend.state]} aria-hidden="true" />{backendLabel(backend)}</span>}
        text={health ? <>weflow {health.version} · API {health.apiVersion}</> : backendDetail(backend)}>
        {health && <span className={'source-name' + (backend.state === 'incompatible' ? ' warning' : '')}>{backend.state === 'incompatible' ? t('不适用于这个网页') : t('适用于这个网页')}</span>}
      </SettingRow>
      {backend.state === 'incompatible' && <p className="setting-warning">{backend.reason}。{backend.advice}</p>}
      {running && <SettingRow name={t('运行方式')} text={running} />}
    </SettingsGroup>
    <SettingsGroup title={t('令牌')} note={tokenSaved
      ? t('这个标签页保存了令牌，关闭后清除。需要更换时在这里粘贴，只保存在这个标签页里。')
      : t('没有保存令牌：从 weflow serve 启动时给出的地址打开网页，可自动连接。需要更换时在这里粘贴，只保存在这个标签页里。')}
      actions={tokenSaved && <button className="secondary-button" type="button" onClick={() => setToken(null)}>{t('清除')}</button>}>
      <TokenForm label={t('新的令牌')} />
    </SettingsGroup>
  </>;
}
