import type { CSSProperties, ReactNode } from 'react';

/** 分段控件：一组选项排在胶囊形的轨道里，滑块滑到选中的那一项下面。 */
export function Segmented({ count, index, className, children, ...rest }: {
  count: number; index: number; className: string; children: ReactNode;
  role?: string; 'aria-label': string;
}) {
  const style = { '--count': count, '--index': index } as CSSProperties;
  return <div className={'segmented ' + className} style={style} {...rest}>
    <span className="segmented-thumb" aria-hidden="true" />{children}
  </div>;
}
