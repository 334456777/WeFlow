// The connection in the settings (#214): the state of weflow serve with a way to check it again, its version and
// whether it suits this page, how it runs, and the token this tab keeps (never its value).
import { useEffect, useState } from 'react';
import { ArrowsClockwise } from '@phosphor-icons/react';
import type { Backend, Health } from '../api/backend';
import { onAuth, setToken, token } from '../api/client';
import { backendDetail, backendLabel } from '../shell/Connection';
import { TokenForm } from '../TokenForm';
import { SettingRow, SettingsGroup } from './layout';
import { t } from '../i18n';

/** How the server runs (#214): its limits and the insight engine, when it says them. */
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

/** The dot of each state, as the sidebar shows it. */
const DOTS: Record<Backend['state'], string> = { checking: 'exporting', ready: 'done', offline: '', incompatible: 'error', unauthorized: 'error' };

export function ConnectionSettings({ backend, recheck }: { backend: Backend; recheck(): void }) {
  const [tokenSaved, setTokenSaved] = useState(() => Boolean(token()));
  useEffect(() => onAuth(event => { if (event === 'token') setTokenSaved(Boolean(token())); }), []);
  const health = 'health' in backend ? backend.health : null;
  const running = health && serverDetail(health);
  return <>
    <SettingsGroup title="weflow serve" actions={<button className="secondary-button" type="button" onClick={recheck}><ArrowsClockwise size={12} />{t('重新检测')}</button>}>
      {/* the state, then the version once the server said it, else what the state means */}
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
