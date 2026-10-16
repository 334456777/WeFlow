import type { CSSProperties, ReactNode } from 'react';

/** Options in a hollow pill with a raised thumb that slides to the selected one. */
export function Segmented({ count, index, className, children, ...rest }: {
  count: number; index: number; className: string; children: ReactNode;
  role?: string; 'aria-label': string;
}) {
  const style = { '--count': count, '--index': index } as CSSProperties;
  return <div className={'segmented ' + className} style={style} {...rest}>
    <span className="segmented-thumb" aria-hidden="true" />{children}
  </div>;
}
