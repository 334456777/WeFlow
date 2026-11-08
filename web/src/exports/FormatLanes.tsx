// The formats of an export written side by side (#628): a lane each, running at once, solid once its file is written.
import type { JobProgress } from '../api/jobs';
import { t } from '../i18n';
import { MESSAGE_FORMATS } from './ExportCards';

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
