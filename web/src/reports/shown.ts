// 报告页当前显示的是哪一份报告（#1077）：操作控制台的保存卡片只保存正在显示的报告，并清掉对上一份报告的提示。
import { useEffect, useRef } from 'react';

/** 用报告自己的键告诉 `onShown` 正在显示的报告；没有报告时、以及报告不再显示时传 null。 */
export function useShownReport(key: string | null, onShown: (key: string | null) => void): void {
  const latest = useRef(onShown);
  latest.current = onShown;
  useEffect(() => {
    latest.current(key);
    return () => latest.current(null);
  }, [key]);
}
