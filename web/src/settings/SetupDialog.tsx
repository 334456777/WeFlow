// 以对话框形式盖在工作区上方的账号配置（#216），离开顶层之前先播放退场动画。
import { useEffect, useRef } from 'react';
import type { Health } from '../api/backend';
import type { Profile } from '../api/setup';
import type { JobsState } from '../jobs/useJobs';
import { usePresence } from '../motion';
import { SetupWizard } from './SetupWizard';
import { t } from '../i18n';

/** `open` 期间在模态对话框里显示的配置向导。 */
export function SetupDialog({ open, profile, jobs, platform, onChanged, onClose, onDone }: {
  open: boolean; profile: Profile | undefined; jobs: JobsState; platform?: Health['platform'];
  onChanged(): void; onClose(): void; onDone(): void;
}) {
  const shown = usePresence(open);
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (shown && dialog && !dialog.open) dialog.showModal?.();
    else if (!shown && dialog?.open) dialog.close();
  }, [shown]);
  return <dialog className="setup-dialog" ref={ref} aria-label={t('账号配置')} data-state={shown?.closing ? 'closing' : 'open'}
    onCancel={event => { event.preventDefault(); onClose(); }}>
    {/* 换了档案（例如刚添加了一个）时，向导从这个档案自己的第一步重新开始（#898） */}
    {shown && <SetupWizard key={profile?.name} profile={profile} jobs={jobs} platform={platform} onChanged={onChanged} onClose={onClose} onDone={onDone} />}
  </dialog>;
}
