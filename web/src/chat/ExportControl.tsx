// 操作控制台里当前会话的导出卡片上的导出按钮（#211），以及按导出作业得出的阶段。
import { ArrowClockwise, CircleNotch, DownloadSimple, X } from '@phosphor-icons/react';
import type { Job } from '../api/jobs';
import type { ExportResult } from '../types';
import { t } from '../i18n';

/** 导出的阶段。 */
export type ExportStage = 'idle' | 'exporting' | 'done' | 'error';

/**
 * 导出按钮：导出期间旁边伸出取消按钮；完成后变成下载文件的链接，旁边的按钮再导出一次。没有选择格式（`ready` 为
 * false）时不能点击，按钮上提示选择格式（#353）。
 */
export function ExportControl({ stage, result, ready, onStart, onCancel }: {
  stage: ExportStage; result: ExportResult | null; ready: boolean; onStart: () => void; onCancel: () => void;
}) {
  const expanded = stage === 'exporting' || (stage === 'done' && result !== null);
  const waiting = !ready && stage !== 'exporting';
  return <div className={'export-control' + (expanded ? ' expanded' : '')}>
    {stage === 'done' && result
      ? <a className="primary-button large" href={result.downloadUrl} download={result.fileName}><span className="morph-label" key="done"><DownloadSimple size={18} />{t('下载文件')}</span></a>
      : <button type="button" className="primary-button large" disabled={waiting} aria-disabled={stage === 'exporting'} onClick={() => { if (stage !== 'exporting') onStart(); }}>
        <span className="morph-label" key={waiting ? 'waiting' : stage}>{stage === 'exporting'
          ? <><CircleNotch size={18} className="spin" />{t('正在导出')}</>
          : waiting ? <><DownloadSimple size={18} />{t('选择导出格式')}</>
          : <><DownloadSimple size={18} />{stage === 'error' ? t('重新导出') : t('开始导出')}</>}</span>
      </button>}
    <button type="button" className="export-side as-button" inert={!expanded} aria-label={stage === 'done' ? t('再次导出') : t('取消导出')} title={stage === 'done' ? t('再次导出') : t('取消导出')} onClick={stage === 'done' ? onStart : onCancel}>
      {stage === 'done' ? <ArrowClockwise size={18} /> : <X size={18} />}
    </button>
  </div>;
}

/** 按卡片的导出作业得出导出的阶段：作业取消后回到 `idle`；没有作业时，有错误为 `error`，否则为 `idle`。 */
export function stageOf(job: Job | undefined, error: string): ExportStage {
  if (!job) return error ? 'error' : 'idle';
  if (job.status === 'queued' || job.status === 'running') return 'exporting';
  return job.status === 'succeeded' ? 'done' : job.status === 'failed' ? 'error' : 'idle';
}
