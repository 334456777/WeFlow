// 同时写出的几种导出格式（#628）：每种一条进度条，一起进行；写完文件的那一条不再闪动，变成实心。
import type { JobProgress } from '../api/jobs';
import { t } from '../i18n';
import { MESSAGE_FORMATS } from './ExportCards';

/** 作业进度里的每种格式一条进度条；只写一种格式的作业（进度里没有 `formats`）不显示。 */
export function FormatLanes({ progress }: { progress: JobProgress | null | undefined }) {
  const formats = progress?.formats;
  if (!formats?.length) return null;
  return <ul className="format-lanes" aria-label={t('各个格式的进度')}>
    {formats.map(format => <li key={format.name} className={format.done ? 'done' : undefined}>
      {MESSAGE_FORMATS.find(f => f.value === format.name)?.label ?? format.name}
      <span className="format-lane-bar" />
    </li>)}
  </ul>;
}
