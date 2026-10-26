// The export button of the open conversation's console card (#211), and where its job stands.
import { ArrowClockwise, CircleNotch, DownloadSimple, X } from '@phosphor-icons/react';
import type { Job } from '../api/jobs';
import type { ExportResult } from '../types';

export type ExportStage = 'idle' | 'exporting' | 'done' | 'error';

/**
 * The export button; while exporting it grows a cancel button, when done it offers the file and another run. Without
 * a format (`ready` false) it cannot be pressed and asks for one (#353).
 */
export function ExportControl({ stage, result, ready, onStart, onCancel }: {
  stage: ExportStage; result: ExportResult | null; ready: boolean; onStart: () => void; onCancel: () => void;
}) {
  const expanded = stage === 'exporting' || (stage === 'done' && result !== null);
  const waiting = !ready && stage !== 'exporting';
  return <div className={'export-control' + (expanded ? ' expanded' : '')}>
    {stage === 'done' && result
      ? <a className="primary-button large" href={result.downloadUrl} download={result.fileName}><span className="morph-label" key="done"><DownloadSimple size={18} />下载文件</span></a>
      : <button type="button" className="primary-button large" disabled={waiting} aria-disabled={stage === 'exporting'} onClick={() => { if (stage !== 'exporting') onStart(); }}>
        <span className="morph-label" key={waiting ? 'waiting' : stage}>{stage === 'exporting'
          ? <><CircleNotch size={18} className="spin" />正在导出</>
          : waiting ? <><DownloadSimple size={18} />选择导出格式</>
          : <><DownloadSimple size={18} />{stage === 'error' ? '重新导出' : '开始导出'}</>}</span>
      </button>}
    <button type="button" className="export-side" inert={!expanded} aria-label={stage === 'done' ? '再次导出' : '取消导出'} title={stage === 'done' ? '再次导出' : '取消导出'} onClick={stage === 'done' ? onStart : onCancel}>
      {stage === 'done' ? <ArrowClockwise size={18} /> : <X size={18} />}
    </button>
  </div>;
}

/** Where the export of the card stands, from its job. */
export function stageOf(job: Job | undefined, error: string): ExportStage {
  if (!job) return error ? 'error' : 'idle';
  if (job.status === 'queued' || job.status === 'running') return 'exporting';
  return job.status === 'succeeded' ? 'done' : job.status === 'failed' ? 'error' : 'idle';
}
