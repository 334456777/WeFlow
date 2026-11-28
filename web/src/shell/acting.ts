// 网页操作的服务端（#896）：回答的那个服务端，或者代替它的示例服务端；示例服务端像服务端一样运行作业、保存设置，
// 改动只保存在页面里。会改动东西或跟随改动的功能（作业、设置）用它。
import { useEffect, useState } from 'react';
import type { Backend, Health } from '../api/backend';
import { get } from '../api/client';

/**
 * 网页操作的服务端的 `/health`：服务端就绪时是它的，示例服务端回答时是示例的（只读一次，就像服务端启动后它的
 * `/health` 不变）；都不是时为 null。
 */
export function useActingHealth(backend: Backend, sample: boolean): Health | null {
  const [sampled, setSampled] = useState<Health | null>(null);
  useEffect(() => {
    if (!sample) { setSampled(null); return; }
    let live = true;
    get<Health>('health').then(health => { if (live) setSampled(health); }, () => { /* 读不到时，设置页一直等它 */ });
    return () => { live = false; };
  }, [sample]);
  return backend.state === 'ready' ? backend.health : sample ? sampled : null;
}
