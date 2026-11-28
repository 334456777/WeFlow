// 网页上显示的与 `weflow serve` 的连接（#221）：侧栏底部和手机标签栏上的圆点和几个字，悬停时和设置里显示它的含义。
import { useEffect, useState } from 'react';
import type { Backend } from '../api/backend';
import { msg, t } from '../i18n';

const labels: Record<Backend['state'], string> = {
  checking: msg('正在连接'), ready: msg('已连接'), offline: msg('未连接'), incompatible: msg('版本不兼容'), unauthorized: msg('需要令牌'),
};

/** 用一两个词写出的状态，与侧栏显示的一样。 */
export const backendLabel = (backend: Backend) => backend.state === 'offline' && backend.held ? t('已断开') : t(labels[backend.state]);

/** 用一句话写出的状态：正在发生什么，该怎么办。 */
export function backendDetail(backend: Backend): string {
  switch (backend.state) {
    case 'checking': return t('正在检查 weflow serve。');
    case 'offline': return backend.held
      ? t('与 weflow serve 的连接断开了，正在重新连接；页面上是断开前读到的记录。')
      : t('没有找到 weflow serve，请先运行 weflow serve。');
    case 'incompatible': return t('{reason}。', { reason: backend.reason });
    case 'unauthorized': return t('weflow serve 在运行，但拒绝了这个标签页的令牌。');
    case 'ready': return t('已连接 weflow serve。');
  }
}

/** 连接保持「已连接」多久（毫秒）之后，提示淡出。 */
const STEADY_MS = 3000;

/** 服务端保持就绪 `STEADY_MS` 之后为 true；变成其他状态时立即重新显示提示。 */
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

/** 接收实时消息时显示「实时」（#201），否则显示连接状态。 */
export function Connection({ backend, realtime }: { backend: Backend; realtime: boolean }) {
  const steady = useSteadyConnection(backend.state);
  return <div className={'connection ' + backend.state + (steady ? ' quiet' : '')} role="status" title={backendDetail(backend)}>
    <span className="connection-dot" /><span className="connection-label">{backend.state === 'ready' && realtime ? t('实时') : backendLabel(backend)}</span>
  </div>;
}
