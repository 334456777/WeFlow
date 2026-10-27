// The connection to `weflow serve` as the page shows it (#221): a dot and a word at the bottom of the sidebar and on
// a phone's tab bar, what it means on hover and in the settings.
import { useEffect, useState } from 'react';
import type { Backend } from '../api/backend';
import { msg, t } from '../i18n';

const labels: Record<Backend['state'], string> = {
  checking: msg('正在连接'), ready: msg('已连接'), offline: msg('未连接'), incompatible: msg('版本不兼容'), unauthorized: msg('需要令牌'),
};

export function backendDetail(backend: Backend): string {
  switch (backend.state) {
    case 'checking': return t('正在检查 weflow serve。');
    case 'offline': return t('没有找到 weflow serve，请先运行 weflow serve。');
    case 'incompatible': return t('{reason}。', { reason: backend.reason });
    case 'unauthorized': return t('weflow serve 在运行，但拒绝了这个标签页的令牌。');
    case 'ready': return t('已连接 weflow serve。');
  }
}

/** How long a connection stays 「已连接」 before its notice fades away. */
const STEADY_MS = 3000;

/** True once the backend has stayed ready for STEADY_MS; any other state shows the notice again at once. */
function useSteadyConnection(state: Backend['state']): boolean {
  const [steady, setSteady] = useState(false);
  useEffect(() => {
    setSteady(false);
    if (state !== 'ready') return;
    const timer = window.setTimeout(() => setSteady(true), STEADY_MS);
    return () => window.clearTimeout(timer);
  }, [state]);
  return steady && state === 'ready';
}

/** 「实时」 while live messages come (#201), otherwise the state of the connection. */
export function Connection({ backend, realtime }: { backend: Backend; realtime: boolean }) {
  const steady = useSteadyConnection(backend.state);
  return <div className={'connection ' + backend.state + (steady ? ' quiet' : '')} role="status" title={backendDetail(backend)}>
    <span className="connection-dot" /><span className="connection-label">{backend.state === 'ready' && realtime ? t('实时') : t(labels[backend.state])}</span>
  </div>;
}
